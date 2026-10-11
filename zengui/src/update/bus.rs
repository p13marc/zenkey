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
use crate::services;
use crate::state::tree::shape_held;
use crate::state::{Deployment, Observation, SubjectState, TreeState, Workspace};

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
    let held = shape_held((&*obs).into(), tick.into(), &obs.watched, &tick.watched);
    // Per tick, not per frame: one bounded lock per *slot* for its key's
    // latency summary (#119, #257 — the fan-out is bounded by the pin
    // count, not the bus). None when unselected, unobserved, or unstamped.
    for slot in sub.all_mut() {
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
    // The Traffic tab (#542): the session's rate every tick, the ranking
    // only while the tab is the one showing.
    work.activity.traffic.tick(
        tick.totals,
        &tick.tree,
        work.activity.tab == crate::message::ActivityTab::Traffic,
    );
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
            obs.seed_totals.history += coverage.history_replies.unwrap_or(0);
            obs.seed_totals.storage += coverage.storage_replies.unwrap_or(0);
            obs.seed_totals.superseded += coverage.superseded;
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
    for slot in sub.all_mut() {
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
        for slot in sub.all_mut() {
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
        .apply_transitions(dep.namespace(), &tick.nodes, now);
    work.verdicts
        .roster
        .refresh(&tick.tree, dep.namespace(), &tick.watched, now);
    // The chart's inputs all advanced above — the history ring, the rate
    // sampler, the facts behind the unit. Rebuilt once per slot here rather
    // than once per frame (#178, #257).
    for slot in sub.all_mut() {
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
    // reports it (#107). This is still the single insert point. No slices
    // since #614 (FL6a): the v1 projection reads "not asked" until FL7.
    let namespace = dep.namespace().to_string();
    dep.facts.ensure(&namespace, key, None);
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
        BusMsg::SessionsOpened(Ok(sessions)) => {
            // New sessions are a new everything: the namespace may differ,
            // so every projection, roster and verdict from the old ones is
            // evidence about a different deployment (O4).
            //
            // This variant has exactly two producers — the launch open and
            // the context switch — and forgetting on the launch one is a
            // no-op over a `Deployment` nothing has written yet. So it can
            // live here, which removes the worst cross-group reach in the
            // file: a pane resetting the app.
            super::deployment::forget(dep, obs, tree, work);
            let services::link::Sessions {
                raw,
                session,
                namespace: _,
            } = sessions;
            dep.raw = Some(raw.clone());
            dep.session = Some(session);
            dep.reseed_store();
            // The context list is read from the shared file, not cached at
            // launch: `zenctl context create` on the other side of the
            // screen should show up here without a restart (#67). As a task
            // (#255): the re-read is right, and a TOML read on the path of
            // every session open — launch and each reconnect — sat a disk
            // access on a frame. Its answer lands on `ContextMsg::Refreshed`.
            let contexts = services::context::refresh();
            dep.schema_store = Some(Arc::new(zenkey_fleet_v1::SchemaStore::new(
                dep.namespace(),
                dep.timeout(),
            )));
            let listing = services::link::list_namespaces(&raw, dep.timeout());
            Task::batch([
                listing,
                contexts,
                start_monitor(dep),
                build_skeleton(dep),
                load_contracts(dep),
            ])
        }
        BusMsg::SessionsOpened(Err(e)) => {
            // A transport that would not come up leaves the contracts
            // answerable (#196): they load alone, and the link still says it
            // failed. A config the user named stays their error — no
            // answering past it.
            let load = if services::link::never_opened(&e) {
                load_contracts(dep)
            } else {
                Task::none()
            };
            obs.link = LinkState::Failed(e);
            load
        }
        BusMsg::NamespacedOpened(namespace, Ok(session)) => {
            // A landing for a namespace the picker has since left is
            // dropped, and its session with it: it would read presence and
            // contracts of a deployment the window no longer names.
            if namespace != dep.namespace() {
                return Task::none();
            }
            dep.session = Some(session);
            dep.reseed_store();
            Task::none()
        }
        BusMsg::NamespacedOpened(namespace, Err(e)) => {
            if namespace != dep.namespace() {
                return Task::none();
            }
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
            // dashboard from it instead of throwing it away (#61). Empty
            // since #614 (FL6a): presence is zk2's, and the nodes pane
            // reads it from FL9.
            work.verdicts.roster.seed(&roster);
            tree.reflatten(dep, obs);
            Task::none()
        }
        BusMsg::SkeletonBuilt(Err(e)) => {
            tracing::warn!("skeleton build failed: {e}");
            Task::none()
        }
        BusMsg::NamespacesListed(Ok(listing)) => {
            dep.namespaces = Some(listing);
            dep.rebuild_namespace_options();
            Task::none()
        }
        BusMsg::NamespacesListed(Err(e)) => {
            tracing::warn!("namespace listing failed: {e}");
            Task::none()
        }
        BusMsg::ContractsLoaded(loaded) => {
            // A load replaces the last one whole: what it dropped must not
            // linger in a store seeded from the old list, so the store is
            // rebuilt from this one (§8.5), and everything judged under the
            // old contracts goes with it.
            dep.contracts = Some(loaded);
            dep.reseed_store();
            forget_judgements(obs, work);
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
            // validating file data against a live bus would put one world's
            // verdicts under the other's data (O4 in miniature).
            schedule_validation(dep, work, &tick)
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
    samples: &[Arc<zenkey_fleet_v1::SampleView>],
    cache: &crate::verdict::VerdictCache,
) -> Vec<(String, String, zenoh::bytes::ZBytes)> {
    // Newest per key, in first-arrival key order: `newest` supersedes,
    // `order` remembers when the key first showed up.
    let mut newest: std::collections::HashMap<&str, &Arc<zenkey_fleet_v1::SampleView>> =
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
    let (Some(raw), Some(store)) = (dep.raw.clone(), dep.schema_store.clone()) else {
        return Task::none();
    };
    let batch = validation_batch(&tick.samples, &work.verdicts.payloads);
    if batch.is_empty() {
        return Task::none();
    }
    services::value::validate(store, raw, None, dep.namespace().to_string(), batch)
}

/// (Re)build the skeleton (#614, FL6a: from no slices, the admin space's
/// declared entities alone, until FL6b builds zk2's from the lens).
pub(crate) fn build_skeleton(dep: &Deployment) -> Task<Message> {
    let Some(raw) = dep.raw.clone() else {
        return Task::none();
    };
    services::sweep::skeleton(raw, dep.namespace().to_string(), dep.timeout())
}

/// Load the contracts the settings name (#614, FL6a) — off the update
/// thread, landing on `ContractsLoaded`.
pub(crate) fn load_contracts(dep: &Deployment) -> Task<Message> {
    services::contracts::load(dep.settings.contracts.clone())
}

pub(crate) fn start_monitor(dep: &Deployment) -> Task<Message> {
    let Some(raw) = dep.raw.clone() else {
        return Task::none();
    };
    // No liveliness on the data pump since #614 (FL6a): presence is zk2's
    // instance and interface tokens, read on the session in the namespace,
    // and v1's alive tokens describe nothing a zk2 bus carries.
    services::watch::start_monitor(raw, Vec::new(), dep.settings.max_keys)
}

/// A contracts (re)load invalidates everything judged under the old ones:
/// the payload verdicts (#164 — the `(key, schema)` discipline, kept by
/// clearing at every door a contract can change through) and the budget
/// join (#221). Both refill on the ordinary cadences; until then "not yet
/// checked" and no-badge are the honest reads.
pub(crate) fn forget_judgements(obs: &mut Observation, work: &mut Workspace) {
    work.verdicts.payloads.clear();
    obs.budgets = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NamespaceChoice;

    /// Both #185 defects, pinned on the namespace picker (#614, FL6a): the
    /// empty namespace is a labelled first-class option, duplicates are
    /// removed whatever order the listing answered in, and the namespace in
    /// force is offered even when the listing did not see it — a picker
    /// whose selected value is not among its options is #187's defect.
    #[test]
    fn namespace_options_are_set_deduped_and_lead_with_the_labelled_bus_root() {
        let mut settings = crate::app::test_settings();
        settings.namespace = "quiet".into();
        let mut dep = Deployment::new(settings);
        let keys: Vec<String> = [
            "acme/zk2/robot1/nav/@zk/instance/0123456789abcdef",
            "spray/zk2/robot1/nav/@zk/instance/0123456789abcdef",
            "acme/zk2/robot2/nav/@zk/instance/fedcba9876543210",
            "zk2/host-a/tc/@zk/instance/0123456789abcdef",
        ]
        .map(String::from)
        .to_vec();
        dep.namespaces = Some(Arc::new(zenkey_fleet::namespaces(
            zenkey_fleet::NAMESPACE_SELECTOR,
            &keys,
            true,
        )));
        dep.rebuild_namespace_options();
        assert_eq!(
            dep.namespace_options,
            [
                NamespaceChoice::bus_root(),
                NamespaceChoice::new("acme"),
                NamespaceChoice::new("spray"),
                NamespaceChoice::new("quiet"),
            ],
            "one row per namespace, the bus root first, the one in force kept"
        );
        assert_eq!(
            dep.namespace_options[0].to_string(),
            "(empty — keys start at zk2/)",
            "the empty namespace is a labelled deployment, never a blank row"
        );
    }

    fn sample(key: &str, payload: &[u8]) -> Arc<zenkey_fleet_v1::SampleView> {
        Arc::new(zenkey_fleet_v1::SampleView {
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
            cache.record(key, zenkey_fleet_v1::Verdict::Valid);
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

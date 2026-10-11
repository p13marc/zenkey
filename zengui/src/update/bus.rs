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
    // Decided *before* the fields below are overwritten (#177). A third
    // rung beside `shape_held`'s two (#614, FL6b): the skeleton the shape
    // was merged from — a declared-only node is shape, and a lens move
    // rebuilds the skeleton without moving a single key.
    // The skeleton is a replay's own while one runs (FL10b).
    let held = shape_held((&*obs).into(), tick.into(), &obs.watched, &tick.watched)
        && tree.merged_from(tree.skeleton(dep));
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
    // The budget window's two observations (#614, FL7), on live ticks only:
    // when each watch was first seen — read before `watched` is replaced
    // below, so an unchanged set costs one pointer compare — and the gap
    // since the last tick.
    if work.replay.replay.is_none() {
        let now = std::time::Instant::now();
        obs.tick_gap.tick(now);
        if !Arc::ptr_eq(&obs.watched, &tick.watched) {
            obs.watch_since
                .retain(|sel, _| tick.watched.iter().any(|w| w == sel));
            for sel in tick.watched.iter() {
                obs.watch_since.entry(sel.clone()).or_insert(now);
            }
        }
    }
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
            obs.seed_totals.add(coverage);
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
        // Each key's zk2 identity, through the lens (#614, FL6a) — on live
        // ticks only: a replayed key resolved through the live lens would put
        // one world's presence under another's keys (FL10b reads replay
        // through an offline lens).
        if work.replay.replay.is_none() {
            dep.ensure_identity(&sample.key);
        }
        work.echo.echo.push(sample);
        // A replayed row that recorded no wire axes (a version-1/2 `.zrec`,
        // FL10b) is recorded as such: its view's axes are not observed.
        let qos_recorded = work
            .replay
            .replay
            .as_ref()
            .is_none_or(|r| r.qos_recorded(sample));
        // History is per-key and costs no subscription of its own: these
        // samples are already flowing for an existing watch (issue #63).
        // Every slot's recorder is fed from this one stream (#257) — a pin
        // costs a bounded ring, never a second subscription — and each
        // recorder keeps only its own key's samples.
        for slot in sub.all_mut() {
            if let Some(rec) = slot.history.as_mut() {
                rec.observe_qos(sample, qos_recorded);
            }
        }
    }
    // The node dashboard (#61; zk2's since #614, FL9): the zero-cost
    // watched-freshness join, per address presence holds — on live ticks
    // only, since presence is the live deployment's and a replayed tree is
    // another world's. Presence itself travels on its own monitor
    // (`BusMsg::Presence`); the descriptors are indexed when the lens's
    // catalog moves (`adopt_lens`), and checked here by pointer.
    if work.replay.replay.is_none() {
        let now = std::time::Instant::now();
        work.verdicts.nodes.refresh(
            dep.presence.as_ref().map(|u| &*u.presence),
            &tick.tree,
            dep.namespace(),
            &tick.watched,
            now,
        );
    }
    work.verdicts.nodes.describe(dep.catalog.as_ref());
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
            dep.session = Some(session.clone());
            dep.reseed_store();
            // The context list is read from the shared file, not cached at
            // launch: `zenctl context create` on the other side of the
            // screen should show up here without a restart (#67). As a task
            // (#255): the re-read is right, and a TOML read on the path of
            // every session open — launch and each reconnect — sat a disk
            // access on a frame. Its answer lands on `ContextMsg::Refreshed`.
            let contexts = services::context::refresh();
            let listing = services::link::list_namespaces(&raw, dep.timeout());
            Task::batch([
                listing,
                contexts,
                start_monitor(dep),
                start_presence(dep, obs),
                read_declared(dep),
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
            tree.reflatten(dep, obs);
            start_presence(dep, obs)
        }
        BusMsg::NamespacedOpened(namespace, Err(e)) => {
            if namespace != dep.namespace() {
                return Task::none();
            }
            obs.link = LinkState::Failed(e);
            Task::none()
        }
        BusMsg::PresenceStarted(namespace, Ok(monitor)) => {
            // Started for a namespace the window has left, or a session that
            // has since gone: shut down, never driven.
            if namespace != dep.namespace() || dep.session.is_none() {
                return services::watch::shutdown(monitor);
            }
            let old = obs.presence_monitor.replace(monitor);
            // A new monitor is a new link: the epoch is its identity.
            obs.presence_epoch += 1;
            old.map(services::watch::shutdown)
                .unwrap_or_else(Task::none)
        }
        BusMsg::PresenceStarted(namespace, Err(e)) => {
            if namespace == dep.namespace() {
                tracing::warn!("the presence monitor would not start: {e}");
            }
            Task::none()
        }
        BusMsg::Presence(epoch, update) => {
            // A landing from a link the window has since restarted describes
            // a session, monitor or store that is gone.
            if epoch != obs.presence_epoch {
                return Task::none();
            }
            let catalog = update.catalog.clone();
            let generation = update.generation;
            dep.presence = Some(update);
            if (generation != dep.lens_generation || !same_catalog(&catalog, &dep.catalog))
                && adopt_lens(dep, work, catalog, generation)
            {
                tree.reflatten(dep, obs);
            }
            Task::none()
        }
        BusMsg::LensOpened(epoch, handle) => {
            if epoch == obs.presence_epoch {
                dep.lens_feed = Some(handle);
            }
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
        BusMsg::DeclaredRead(Ok(declared)) => {
            dep.admin_declared = declared;
            if dep.refresh_skeleton() {
                tree.reflatten(dep, obs);
            }
            Task::none()
        }
        BusMsg::DeclaredRead(Err(e)) => {
            // Not an empty admin space: the skeleton keeps saying "admin
            // n/a" (O4), and the tree is the lens's declared keyspace and
            // what was observed.
            tracing::warn!("admin space read failed: {e}");
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
            // The same revisions as held (every session open reloads them):
            // the store, the lens and every verdict stand, and only the
            // load's own account — its paths and problems — is replaced.
            let unchanged = dep
                .contracts
                .as_ref()
                .is_some_and(|held| same_revisions(&held.set, &loaded.set));
            dep.contracts = Some(loaded);
            if unchanged {
                dep.recount_contracts();
                return Task::none();
            }
            dep.reseed_store();
            forget_judgements(obs, work);
            // The skeleton moved with the contracts (`reseed_store`).
            tree.reflatten(dep, obs);
            // The lens feed retrieves into the store it was opened on: the
            // presence link restarts over the new one.
            restart_presence(obs);
            Task::none()
        }
        BusMsg::Link(state) => {
            obs.link = state;
            Task::none()
        }
        BusMsg::Tick(tick) => {
            // The lens first (#614, FL6a): a background re-read a nudge set
            // off swaps the feed's catalog without a presence move, and the
            // samples below resolve through whatever it holds now.
            poll_lens(dep, work);
            apply_tick(dep, obs, sub, tree, work, &tick);
            // Live ticks only, deliberately: replay feeds `apply_tick` from
            // a file (`update/pane/replay.rs`) and never reaches this arm —
            // validating file data against a live bus, or judging a live
            // budget under file keys, would put one world's verdicts under
            // the other's data (O4 in miniature). A replay validates through
            // its own offline lens instead (FL10b).
            Task::batch([
                schedule_validation(dep, work, &tick),
                schedule_budget(dep, obs, work),
            ])
        }
        BusMsg::VerdictsChecked(batch) => {
            // A live batch landing while a replay runs is the live world's
            // (FL10b): the cache holds the capture's verdicts, checked
            // offline, and lands those on the replay's own message.
            if work.replay.replay.is_some() {
                return Task::none();
            }
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
pub(crate) fn validation_batch(
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
    let batch = validation_batch(&tick.samples, &work.verdicts.payloads);
    if batch.is_empty() {
        return Task::none();
    }
    // Through the lens (#614, FL6a): session-free, so a check needs no
    // session of its own — only what the lens resolves with.
    services::value::validate(dep.lens_inputs(), batch)
}

/// Every this-many ticks (~4 s), judge the budgets again (#221; zk2's since
/// #614, FL7). Throttled: the judgement walks every placed family's subtree,
/// and a population does not explode per frame. Only with a presence read
/// — without one no owner can be present, and `obs.budgets` keeps what it
/// last said, or `None`, which draws no badge.
const BUDGET_EVERY_TICKS: u64 = 16;

fn schedule_budget(dep: &Deployment, obs: &Observation, work: &Workspace) -> Task<Message> {
    // The verdict cache's logical clock is the tick count — one counter,
    // advanced in `apply_tick`, shared by both throttles.
    if work.verdicts.payloads.tick_count() % BUDGET_EVERY_TICKS != 1 {
        return Task::none();
    }
    let Some(catalog) = dep.catalog.clone() else {
        return Task::none();
    };
    let now = std::time::Instant::now();
    let dropped = obs.monitor.as_ref().map_or(0, |m| m.core().dropped());
    services::sweep::budget(crate::budget::BudgetInputs {
        namespace: dep.namespace().to_string(),
        catalog: Some(catalog),
        store: dep.store.clone(),
        presence: dep.presence.as_ref().map(|u| Arc::clone(&u.presence)),
        observed: Arc::clone(&obs.observed),
        watches: obs
            .watch_since
            .iter()
            .map(|(sel, since)| (sel.clone(), *since))
            .collect(),
        reader: crate::budget::ReaderLoss {
            // What the reader lost (core §2.7, 0.25): the broadcast's drops
            // and the key table's evictions, since the monitor started — a
            // superset of what any one window lost, which is the side a
            // "lossless" must err on.
            lost: dropped + obs.keys_evicted,
            gap: obs.tick_gap.longest(now),
        },
        timeout: dep.timeout(),
        now,
        wall: std::time::SystemTime::now(),
    })
}

/// Whether two contract sets hold the same revisions, by `(iface,
/// fingerprint)`: a bundle is content-addressed, so equal keys are equal
/// contracts.
fn same_revisions(a: &zenkey_fleet::ContractSet, b: &zenkey_fleet::ContractSet) -> bool {
    let keys = |s: &zenkey_fleet::ContractSet| {
        s.iter()
            .map(|r| (r.iface().clone(), r.fingerprint().clone()))
            .collect::<std::collections::BTreeSet<_>>()
    };
    keys(a) == keys(b)
}

/// Whether two catalog handles are the same catalog — the feed swaps an
/// `Arc`, so identity is the cheap and exact question.
fn same_catalog(
    a: &Option<Arc<zenkey_fleet::Catalog>>,
    b: &Option<Arc<zenkey_fleet::Catalog>>,
) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

/// The lens moved (#614, FL6a): a new catalog, or new answers in the store.
/// Every identity and every verdict judged before is due again — every
/// identity is resolved again now (FL7), and the verdict cache keeps each
/// badge on screen as the last true reading until its recheck lands.
/// The skeleton is rebuilt from the moved lens (FL6b) — the one door it is
/// rebuilt through besides the contracts and the admin space — and `true`
/// says it was, so a landing that no tick follows can redraw the tree.
fn adopt_lens(
    dep: &mut Deployment,
    work: &mut Workspace,
    catalog: Option<Arc<zenkey_fleet::Catalog>>,
    generation: u64,
) -> bool {
    dep.catalog = catalog;
    dep.lens_generation = generation;
    dep.reresolve_identities();
    // The nodes pane's descriptors are the catalog's (#614, FL9).
    work.verdicts.nodes.describe(dep.catalog.as_ref());
    work.verdicts.payloads.invalidate();
    dep.recount_contracts();
    dep.refresh_skeleton()
}

/// Each tick, ask the lens feed whether it moved without presence moving
/// (a background re-read a nudge set off, a retrieval landing): its catalog
/// pointer-compared, its generation read (#614, FL4's note for FL6a).
fn poll_lens(dep: &mut Deployment, work: &mut Workspace) {
    let Some(feed) = dep.lens_feed.as_ref() else {
        return;
    };
    let catalog = feed.0.catalog();
    let generation = feed.0.generation();
    if generation != dep.lens_generation || !same_catalog(&catalog, &dep.catalog) {
        // The tree follows in this tick's `apply_tick`: the skeleton rung of
        // its shape check sees the rebuilt skeleton.
        adopt_lens(dep, work, catalog, generation);
    }
}

/// Start the presence monitor on the session in the namespace (#614,
/// FL6a), shutting down the one before it: its landing bumps the presence
/// epoch, which is what starts the presence link.
pub(crate) fn start_presence(dep: &Deployment, obs: &mut Observation) -> Task<Message> {
    let old = obs
        .presence_monitor
        .take()
        .map(services::watch::shutdown)
        .unwrap_or_else(Task::none);
    obs.presence_epoch += 1;
    let Some(session) = dep.session.clone() else {
        return old;
    };
    Task::batch([
        old,
        services::watch::start_presence(session, dep.namespace().to_string()),
    ])
}

/// Restart the presence link over the same monitor (#614, FL6a): the
/// contract store under the lens was rebuilt, or the deployment forgot what
/// the link had told it. A new epoch is a new stream, reseeded. With no
/// presence monitor there is no link to restart, and the epoch stays.
pub(crate) fn restart_presence(obs: &mut Observation) {
    if obs.presence_monitor.is_some() {
        obs.presence_epoch += 1;
    }
}

/// Ask the admin space what sessions declared (#614, FL6b): the
/// skeleton's one input that costs a query, on the session in no namespace.
pub(crate) fn read_declared(dep: &Deployment) -> Task<Message> {
    let Some(raw) = dep.raw.clone() else {
        return Task::none();
    };
    services::sweep::declared(raw, dep.timeout())
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
    services::watch::start_monitor(raw, dep.settings.max_keys)
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
            cache.record(key, zenkey_fleet::report::Conformance::Valid);
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

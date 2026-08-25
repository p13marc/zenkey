//! Pointing the window at a different fleet (#175).
//!
//! [`update`] names **all six** sub-states, and the number *is* the finding:
//! the issue assumed a base change could be one field replacement. It clears
//! four, remembers a preference in `chrome`, and drops the selected key's
//! decode in `sub` — a rendering of a value fetched from a fleet this window
//! is no longer pointed at.
//!
//! [`forget`] is what a base change owes the old fleet — four calls, of which
//! two delegate, one is a struct replacement, and exactly one line reaches
//! across a group boundary.

use std::sync::Arc;

use iced::Task;

use crate::message::{DeploymentMsg, LinkState, Message};
use crate::services;
use crate::state::{Chrome, Deployment, Observation, SubjectState, TreeState, Workspace};

/// What the app is pointed at, and the coverage that follows.
pub(crate) fn update(
    chrome: &mut Chrome,
    dep: &mut Deployment,
    obs: &mut Observation,
    sub: &mut SubjectState,
    tree: &mut TreeState,
    work: &mut Workspace,
    msg: DeploymentMsg,
) -> Task<Message> {
    match msg {
        DeploymentMsg::ScopeWatchesStarted(ids) => {
            for id in &ids {
                obs.seeding.insert(*id, None);
            }
            obs.scope_watches = ids;
            Task::none()
        }
        DeploymentMsg::ScopeWatchesReleased(Ok(())) => Task::none(),
        DeploymentMsg::ScopeWatchesReleased(Err(e)) => {
            tracing::warn!("releasing the scope watches failed: {e}");
            Task::none()
        }
        DeploymentMsg::ContextApplied { name, stored } => {
            apply_context(dep, *stored);
            chrome.prefs.context = name;
            super::chrome::remember(chrome, dep, work);
            reopen_session(dep, obs)
        }
        DeploymentMsg::ScopeWatchToggled => {
            if obs.scope_watches.is_empty() {
                watch_scope(dep, obs)
            } else {
                unwatch_scope(obs)
            }
        }
        DeploymentMsg::BaseSelected(base) => {
            if base == dep.base() {
                return Task::none();
            }
            dep.settings.base = base;
            // The base is an input to every projection and to the
            // skeleton, and watch selectors are base-relative: a fresh
            // monitor is the obviously-correct restart. Everything the old
            // base taught us is evidence about a different deployment (O4).
            repoint(dep, obs, sub, tree, work)
        }
        DeploymentMsg::ScopeSelected(scope) => {
            if scope == dep.settings.scope {
                return Task::none();
            }
            // Picking custom with nothing typed yet forks the selectors the
            // window is already using (#187): the preset's resolved set, via
            // `scope::selectors` — the one place selectors are built, so the
            // Deployment preset's explicit @catalog line (RFC 03 §4 D4)
            // survives the fork.
            if scope == crate::scope::ScopePreset::Custom && dep.settings.selectors.is_empty() {
                dep.settings.selectors = dep.settings.scope.selectors(dep.base(), &[]);
            }
            dep.settings.scope = scope;
            // Remembered for the next launch (issue #73; selectors too, #187).
            super::chrome::remember(chrome, dep, work);
            let mut tasks = Vec::new();
            // If the scope is being observed, re-point the observation.
            if !obs.scope_watches.is_empty() {
                tasks.push(unwatch_scope(obs));
                tasks.push(watch_scope(dep, obs));
            }
            // A custom scope is the user's own key expressions, so picking it
            // opens the editor on them — through the same Open message the
            // location bar's chip sends, which is what seeds the draft.
            if scope == crate::scope::ScopePreset::Custom {
                tasks.push(Task::done(Message::Chrome(
                    crate::message::ChromeMsg::Palette(crate::view::palette::PaletteMsg::Open(
                        crate::view::palette::Overlay::Selectors,
                    )),
                )));
            }
            Task::batch(tasks)
        }
        DeploymentMsg::CustomSelectorsApplied(rows) => {
            work.bench.scope_form.status = Some(Ok(format!(
                "{} applied — the scope is custom, and it survives a restart",
                crate::view::kit::plural(rows.len(), "selector"),
            )));
            dep.settings.scope = crate::scope::ScopePreset::Custom;
            dep.settings.selectors = rows;
            super::chrome::remember(chrome, dep, work);
            // If the scope is being observed, re-point the observation at
            // the new selectors — the tail `ScopeSelected` shares.
            if !obs.scope_watches.is_empty() {
                let release = unwatch_scope(obs);
                let acquire = watch_scope(dep, obs);
                return Task::batch([release, acquire]);
            }
            Task::none()
        }
        DeploymentMsg::TuningApplied(t) => {
            let mut applied: Vec<String> = Vec::new();
            if t.echo_lines != dep.settings.echo_lines {
                dep.settings.echo_lines = t.echo_lines;
                // Live, no reconnect (#188): the ring re-bounds in place,
                // and its resize keeps every loss counter — raising a bound
                // never un-loses what the old bound cost.
                work.echo.echo.resize(t.echo_lines);
                applied.push(format!("echo ring {} lines (live)", t.echo_lines));
            }
            if t.history_entries != dep.settings.history_entries {
                dep.settings.history_entries = t.history_entries;
                // Every recorder in flight resizes too — the pins' as much
                // as the follow slot's (#257); the next selection starts at
                // the new bound anyway.
                for slot in sub.all_mut() {
                    if let Some(rec) = slot.history.as_mut() {
                        rec.ring.resize(t.history_entries);
                    }
                }
                applied.push(format!("history {} entries (live)", t.history_entries));
            }
            if t.timeout_secs != dep.settings.timeout_secs {
                dep.settings.timeout_secs = t.timeout_secs;
                applied.push(format!("timeout {}s (from the next query)", t.timeout_secs));
            }
            if t.max_keys != dep.settings.max_keys {
                dep.settings.max_keys = t.max_keys;
                applied.push(format!("max keys {} (on reconnect)", t.max_keys));
            }
            if t.eager != dep.settings.eager {
                dep.settings.eager = t.eager;
                applied.push(format!(
                    "eager {} (on reconnect)",
                    if t.eager { "on" } else { "off" }
                ));
            }
            let registry_changed = t.registry != dep.settings.registry;
            if registry_changed {
                dep.settings.registry = t.registry;
                applied.push(
                    "registry changed — re-pointing: verdicts about the old slices are dropped"
                        .to_string(),
                );
            }
            work.bench.settings_form.status = Some(Ok(if applied.is_empty() {
                "nothing changed".to_string()
            } else {
                applied.join(" · ")
            }));
            // The overlay's knobs are remembered (#188) — written on the
            // settle timer like a splitter drag, and only from here: a
            // command-line flag never becomes the new default by itself.
            chrome.prefs.echo_lines = Some(dep.settings.echo_lines);
            chrome.prefs.history_entries = Some(dep.settings.history_entries);
            chrome.prefs.max_keys = Some(dep.settings.max_keys);
            chrome.prefs.eager = Some(dep.settings.eager);
            chrome.prefs_dirty = true;
            if registry_changed {
                // The registry is a `SliceSource` input: changing it re-runs
                // the union and can change every registration badge in the
                // tree — so it takes the same forget path a base change
                // does, rather than layering new slices over old verdicts.
                return repoint(dep, obs, sub, tree, work);
            }
            Task::none()
        }
        DeploymentMsg::Reconnect => {
            forget(dep, obs, tree, work);
            super::bus::start_monitor(dep)
        }
    }
}

/// Point the session at different coverage: forget the old deployment's
/// evidence, rebuild the schema store, and restart the monitor and the slice
/// load. The shared tail of a base change and a registry change (#188) — the
/// latter goes through the *same* path deliberately, because new slices over
/// old verdicts is how a stale registration badge survives a registry swap.
fn repoint(
    dep: &mut Deployment,
    obs: &mut Observation,
    sub: &mut SubjectState,
    tree: &mut TreeState,
    work: &mut Workspace,
) -> Task<Message> {
    forget(dep, obs, tree, work);
    dep.schema_store = Some(Arc::new(zenkey_fleet::SchemaStore::new(
        dep.base(),
        dep.timeout(),
    )));
    // Every slot's decode was judged under the departing deployment's
    // schemas (#257) — the pins' as much as the follow slot's.
    for slot in sub.all_mut() {
        slot.decoded = None;
    }
    tree.reflatten(dep, obs);
    Task::batch([super::bus::start_monitor(dep), super::bus::load_slices(dep)])
}

/// Everything a session learned about one deployment, forgotten.
///
/// Factored out because three paths need exactly this — a base change, a
/// reconnect, and a context switch — and each one that forgot a different
/// subset would leave a stale verdict on screen about a fleet it is no
/// longer looking at (O4).
///
/// What survives, deliberately (#109 audit): the tree selection, the
/// fetched value, the history recorder, and the call/publish forms —
/// each keyed by a full wire key that names its own base, or user input
/// rather than projected evidence. What cannot be cleared here — a task
/// already in flight — is judged at its landing instead: doctor, blob
/// probe/fetch and node_info each carry the base they ran against.
/// Point at a different fleet, and stop claiming anything about the old
/// one.
///
/// Four of the six sub-states, and that number is the finding: the issue
/// assumed this could be one field replacement. Two of the four delegate,
/// one is a struct replacement, and exactly one line reaches across a
/// group boundary — `expanded`, because a stale path re-expands a
/// coincidentally matching new subtree (#179).
pub(crate) fn forget(
    dep: &mut Deployment,
    obs: &mut Observation,
    tree: &mut TreeState,
    work: &mut Workspace,
) {
    dep.pointed_at();
    obs.forget_coverage();
    work.verdicts.forget();
    tree.expanded.clear();
}

/// Layer a stored context over the live settings — the same precedence
/// `Cli::settings_with` applies, minus the flags, because a context picked
/// in-app *is* the explicit choice.
fn apply_context(dep: &mut Deployment, stored: zenkey_explorer_config::StoredContext) {
    dep.settings.base = stored.base.unwrap_or_default();
    dep.settings.connect = stored.connect;
    dep.settings.listen = stored.listen;
    dep.settings.scouting = stored.scouting;
    dep.settings.zenoh_config = stored.zenoh_config;
    if !stored.registry.is_empty() {
        dep.settings.registry = stored.registry;
    }
    if let Some(t) = stored.timeout {
        dep.settings.timeout_secs = t;
    }
}

/// Tear the link down and build a new one on the current settings.
///
/// The epoch bump the subscription machinery already does on
/// `MonitorStarted` is what retires the old pump; nothing here has to
/// coordinate with it.
fn reopen_session(dep: &mut Deployment, obs: &mut Observation) -> Task<Message> {
    obs.link = LinkState::Connecting;
    // The old pump goes down *acknowledged* — the next monitor declares over
    // the same keys, and a dropped handle only aborts the tasks while its
    // subscribers undeclare in the background.
    let teardown = obs
        .monitor
        .take()
        .map(services::watch::shutdown)
        .unwrap_or_else(Task::none);
    dep.session = None;
    Task::batch([
        teardown,
        services::link::reopen(
            dep.settings.zenoh_config.clone(),
            dep.settings.connect.clone(),
            dep.settings.listen.clone(),
            dep.settings.scouting,
        ),
    ])
}

pub(crate) fn watch_scope(dep: &Deployment, obs: &Observation) -> Task<Message> {
    let Some(monitor) = obs.monitor.clone() else {
        return Task::none();
    };
    let selectors = dep
        .settings
        .scope
        .selectors(dep.base(), &dep.settings.selectors);
    // Scope watches seed too (issue #92) — the eager preset is "observe
    // this scope", and current state is part of observing it.
    let policy = zenkey_fleet::SeedPolicy {
        timeout: dep.timeout(),
        ..Default::default()
    };
    services::watch::scope(monitor, selectors, policy)
}

fn unwatch_scope(obs: &mut Observation) -> Task<Message> {
    let Some(monitor) = obs.monitor.clone() else {
        return Task::none();
    };
    let ids = std::mem::take(&mut obs.scope_watches);
    for id in &ids {
        obs.seeding.remove(id);
    }
    services::watch::release_scope(monitor, ids)
}

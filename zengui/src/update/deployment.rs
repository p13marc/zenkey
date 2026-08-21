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
use crate::state::{Chrome, Deployment, Observation, Subject, TreeState, Workspace};

/// What the app is pointed at, and the coverage that follows.
pub(crate) fn update(
    chrome: &mut Chrome,
    dep: &mut Deployment,
    obs: &mut Observation,
    sub: &mut Subject,
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
            forget(dep, obs, tree, work);
            dep.schema_store = Some(Arc::new(zenkey_fleet::decode::SchemaStore::new(
                dep.base(),
                dep.timeout(),
            )));
            sub.decoded = None;
            tree.reflatten(dep, obs);
            Task::batch([super::bus::start_monitor(dep), super::bus::load_slices(dep)])
        }
        DeploymentMsg::ScopeSelected(scope) => {
            if scope == dep.settings.scope {
                return Task::none();
            }
            dep.settings.scope = scope;
            // Remembered for the next launch (issue #73).
            super::chrome::remember(chrome, dep, work);
            // If the scope is being observed, re-point the observation.
            if !obs.scope_watches.is_empty() {
                let release = unwatch_scope(obs);
                let acquire = watch_scope(dep, obs);
                return Task::batch([release, acquire]);
            }
            Task::none()
        }
        DeploymentMsg::Reconnect => {
            forget(dep, obs, tree, work);
            super::bus::start_monitor(dep)
        }
    }
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
fn apply_context(dep: &mut Deployment, stored: zenkey_fleet::StoredContext) {
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
    obs.monitor = None;
    dep.session = None;
    services::link::reopen(
        dep.settings.zenoh_config.clone(),
        dep.settings.connect.clone(),
        dep.settings.listen.clone(),
        dep.settings.scouting,
    )
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

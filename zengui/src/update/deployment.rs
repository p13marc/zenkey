//! Pointing the window at a different fleet (#175).
//!
//! [`update`] names **all six** sub-states, and the number *is* the finding:
//! the issue assumed a base change could be one field replacement. It clears
//! four, remembers a preference in `chrome`, and drops the selected key's
//! decode in `sub` — a rendering of a value fetched from a fleet this window
//! is no longer pointed at.
//!
//! [`forget`] is what a namespace change owes the old fleet — four calls, of
//! which two delegate, one is a struct replacement, and exactly one line
//! reaches across a group boundary.

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
            // What is on screen now is the context's namespace, and the one
            // the next launch should come back to (#614, FL6a).
            chrome.prefs.namespace = Some(dep.settings.namespace.clone());
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
        DeploymentMsg::NamespaceSelected(namespace) => {
            if namespace == dep.namespace() {
                return Task::none();
            }
            dep.settings.namespace = namespace;
            // Remembered from the picker, and only from it (#614, FL6a): a
            // `--namespace` flag wins its launch without becoming the next
            // one's default.
            chrome.prefs.namespace = Some(dep.settings.namespace.clone());
            chrome.prefs_dirty = true;
            // The namespace is an input to every projection, the skeleton
            // and every preset's wire selectors: the data pump restarts on
            // the session in no namespace, and the session in the namespace
            // is reopened. Everything the old namespace taught us is
            // evidence about a different deployment (O4).
            repoint(dep, obs, sub, tree, work)
        }
        DeploymentMsg::ScopeSelected(scope) => {
            if scope == dep.settings.scope {
                return Task::none();
            }
            // Picking custom with nothing typed yet forks the selectors the
            // window is already using (#187): the preset's resolved set, via
            // `scope::selectors` — the one place selectors are built, so the
            // explicit preset's named verbatim kinds survive the fork.
            if scope == crate::scope::ScopePreset::Custom && dep.settings.selectors.is_empty() {
                dep.settings.selectors = dep.settings.scope.selectors(dep.namespace(), &[]);
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
            // The declared entities (#890): asked at once when turned on,
            // and forgotten when turned off, so "off" is what the tree shows
            // too — the contract skeleton and what was observed.
            let ask_declared = t.admin_declared && !dep.settings.admin_declared;
            if t.admin_declared != dep.settings.admin_declared {
                dep.settings.admin_declared = t.admin_declared;
                if t.admin_declared {
                    applied.push("declared entities on — asked now (#890)".to_string());
                } else {
                    applied.push("declared entities off — no longer asked (#890)".to_string());
                    dep.admin_declared = None;
                    if dep.refresh_skeleton() {
                        tree.reflatten(dep, obs);
                    }
                }
            }
            let contracts_changed = t.contracts != dep.settings.contracts;
            if contracts_changed {
                dep.settings.contracts = t.contracts;
                applied.push(
                    "contracts changed — reloading: every verdict judged under the old \
                     ones is dropped"
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
            // The contracts too (#614, FL6a), in this window's own
            // preferences and never in the shared context.
            chrome.prefs.echo_lines = Some(dep.settings.echo_lines);
            chrome.prefs.history_entries = Some(dep.settings.history_entries);
            chrome.prefs.max_keys = Some(dep.settings.max_keys);
            chrome.prefs.eager = Some(dep.settings.eager);
            chrome.prefs.admin_declared = Some(dep.settings.admin_declared);
            chrome.prefs.contracts = dep.settings.contracts.clone();
            chrome.prefs_dirty = true;
            let declared = if ask_declared {
                super::bus::read_declared(dep)
            } else {
                Task::none()
            };
            if contracts_changed {
                // A load replaces the contracts whole, and its landing
                // rebuilds the store and drops what was judged under the
                // old ones — the decode on screen included, which described
                // a payload under contracts no longer held.
                for slot in sub.all_mut() {
                    slot.decoded = None;
                }
                return Task::batch([super::bus::load_contracts(dep), declared]);
            }
            declared
        }
        DeploymentMsg::Reconnect => {
            forget(dep, obs, tree, work);
            // The forget dropped what the presence link told the deployment
            // (its projection, the lens, the store): the link restarts over
            // the same presence monitor, reseeded.
            super::bus::restart_presence(obs);
            // The admin space's answer went with the forget, and the
            // skeleton says "admin n/a" until it is asked again.
            Task::batch([
                super::bus::start_monitor(dep),
                super::bus::read_declared(dep),
            ])
        }
        DeploymentMsg::RereadPresence => {
            // A revision no holder served is asked again by the lens that
            // opens next; a verified one is content-addressed and kept.
            dep.store.forget_absences();
            dep.recount_contracts();
            // The link restarts over the same monitor and store: a fresh
            // seed and a fresh lens read. What they held stays on screen
            // until their first landing replaces it — a re-read, not a
            // forget.
            super::bus::restart_presence(obs);
            Task::none()
        }
    }
}

/// Point the window at another namespace: forget the old deployment's
/// evidence, restart the data pump on the session in no namespace, and
/// reopen the session in the namespace (#614, FL6a) — the one a namespace
/// actually changes.
fn repoint(
    dep: &mut Deployment,
    obs: &mut Observation,
    sub: &mut SubjectState,
    tree: &mut TreeState,
    work: &mut Workspace,
) -> Task<Message> {
    // The departing namespace's session goes before the forget, so nothing
    // carries it across: presence and contracts read through it describe
    // the deployment the window is leaving. Its presence monitor goes with
    // it, and the epoch moves, so the link over them stops.
    dep.session = None;
    let presence = obs
        .presence_monitor
        .take()
        .map(services::watch::shutdown)
        .unwrap_or_else(Task::none);
    obs.presence_epoch += 1;
    forget(dep, obs, tree, work);
    // Every slot's decode was judged under the departing deployment's
    // contracts (#257) — the pins' as much as the follow slot's.
    for slot in sub.all_mut() {
        slot.decoded = None;
    }
    tree.reflatten(dep, obs);
    let reopen = if dep.raw.is_some() {
        services::link::open_namespaced(
            dep.namespace().to_string(),
            services::link::Transport::of(&dep.settings),
        )
    } else {
        Task::none()
    };
    Task::batch([
        presence,
        super::bus::start_monitor(dep),
        super::bus::read_declared(dep),
        reopen,
    ])
}

/// Everything a session learned about one deployment, forgotten.
///
/// Factored out because three paths need exactly this — a namespace change,
/// a reconnect, and a context switch — and each one that forgot a different
/// subset would leave a stale verdict on screen about a fleet it is no
/// longer looking at (O4).
///
/// What survives, deliberately (#109 audit): the tree selection, the
/// fetched value, the history recorder, and the call/publish forms —
/// each keyed by a full wire key that names its own namespace, or user
/// input rather than projected evidence. What cannot be cleared here — a
/// task already in flight — is judged at its landing instead: the doctor
/// run and an address's health reading each carry the namespace they ran
/// against.
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
/// in-app *is* the explicit choice. Its `registry` is v1's and is read by
/// nothing (#614, FL6a); a save carries it back to the store untouched.
fn apply_context(dep: &mut Deployment, stored: zenkey_explorer_config::StoredContext) {
    dep.settings.namespace = stored.base.unwrap_or_default();
    dep.settings.connect = stored.connect;
    dep.settings.listen = stored.listen;
    dep.settings.scouting = stored.scouting;
    dep.settings.zenoh_config = stored.zenoh_config;
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
    // The presence monitor rides the session in the namespace, which goes.
    let presence = obs
        .presence_monitor
        .take()
        .map(services::watch::shutdown)
        .unwrap_or_else(Task::none);
    obs.presence_epoch += 1;
    dep.raw = None;
    dep.session = None;
    Task::batch([
        teardown,
        presence,
        services::link::reopen(
            dep.namespace().to_string(),
            services::link::Transport::of(&dep.settings),
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
        .selectors(dep.namespace(), &dep.settings.selectors);
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

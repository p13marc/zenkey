//! Dynamic shell completion from the slice cache (issue #54).
//!
//! §6.1 decided this ("clap_complete static + dynamic — subject/producer/type
//! names from the cache") and the static half shipped; the dynamic half never
//! did, because the *cache was never written*. `SliceSet::write_cache` and
//! `read_cache` had existed, unused, since the engine extraction.
//!
//! Three properties this must have, in descending order of how badly getting
//! them wrong would hurt:
//!
//! 1. **Never touch the bus.** A `<TAB>` press must not open a session, and
//!    certainly must not fan out an `introspect` sweep — a completion that
//!    queries a fleet is a completion that hangs when the fleet is down.
//!    Everything here reads files.
//! 2. **Never fail.** A missing, stale or unreadable cache yields *no*
//!    candidates, which degrades to clap's static completion. A panic in a
//!    completion hook is a broken shell.
//! 3. **Never claim.** These are names from the last sighting, not a live
//!    inventory. Nothing here renders as a fact about the bus (O4) — which is
//!    also why `zenctl cache show|refresh|clear` exists: a tool that starts
//!    leaving files on somebody's disk owes them a way to see and delete them.

use anyhow::{Result, anyhow};
use clap_complete::CompletionCandidate;
use zenkey_fleet::SliceSet;

/// The environment variable the generated script sets when it calls back.
const COMPLETE_VAR: &str = "ZENCTL_COMPLETE";

/// Serve one completion request, if this invocation *is* one.
///
/// Called before `Cli::parse()`: the shell runs `ZENCTL_COMPLETE=<shell>
/// zenctl …` and expects candidates on stdout, so argument parsing must not
/// happen (and must not fail) first. A no-op otherwise.
pub fn maybe_serve() {
    use clap::CommandFactory as _;
    clap_complete::CompleteEnv::with_factory(crate::Cli::command)
        .var(COMPLETE_VAR)
        .bin("zenctl")
        .complete();
}

/// The shell snippet that registers the dynamic completer.
pub fn registration(shell: clap_complete::Shell) -> Result<()> {
    let shells = clap_complete::env::Shells::builtins();
    let completer = shells
        .completer(&shell.to_string())
        .ok_or_else(|| anyhow!("no dynamic completer for {shell}; try --static"))?;
    let bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(str::to_string))
        .unwrap_or_else(|| "zenctl".to_string());
    completer.write_registration(
        COMPLETE_VAR,
        "zenctl",
        "zenctl",
        &bin,
        &mut std::io::stdout(),
    )?;
    Ok(())
}

/// The cached slices for the context this invocation would use.
///
/// Deliberately infallible: completion has no error channel that a user would
/// want to read mid-keystroke.
fn cached() -> SliceSet {
    let name = zenkey_fleet::active_name(None);
    SliceSet::read_cache(&zenkey_fleet::cache_dir(name.as_deref()))
}

fn candidates(values: impl IntoIterator<Item = String>) -> Vec<CompletionCandidate> {
    let mut values: Vec<String> = values.into_iter().collect();
    values.sort();
    values.dedup();
    values.into_iter().map(CompletionCandidate::new).collect()
}

/// Producer (and service) names.
pub fn producers() -> Vec<CompletionCandidate> {
    producers_in(&cached())
}

fn producers_in(set: &SliceSet) -> Vec<CompletionCandidate> {
    candidates(set.slices().iter().map(|s| s.name.clone()))
}

/// Declared payload type names, from every binding site.
pub fn types() -> Vec<CompletionCandidate> {
    types_in(&cached())
}

fn types_in(set: &SliceSet) -> Vec<CompletionCandidate> {
    candidates(set.slices().iter().flat_map(|s| {
        s.subjects
            .iter()
            .map(|d| d.type_name.clone())
            .filter(|t| !t.is_empty())
            .chain(s.procedures.iter().filter_map(|p| p.reply.clone()))
            .chain(s.procedures.iter().filter_map(|p| p.request.clone()))
            .chain(s.blob.iter().filter_map(|b| b.reference.clone()))
            .collect::<Vec<_>>()
    }))
}

/// Procedure paths, across producers.
pub fn procedures() -> Vec<CompletionCandidate> {
    procedures_in(&cached())
}

fn procedures_in(set: &SliceSet) -> Vec<CompletionCandidate> {
    candidates(
        set.slices()
            .iter()
            .flat_map(|s| s.procedures.iter().map(|p| p.path.clone()))
            .collect::<Vec<_>>(),
    )
}

/// The three classes — a closed vocabulary (RFC 04 §1), so this one is exact
/// rather than cached.
pub fn classes() -> Vec<CompletionCandidate> {
    candidates(
        ["telemetry", "state", "events"]
            .into_iter()
            .map(str::to_string),
    )
}

/// The five QoS profiles — likewise closed (RFC 04 §3), read off the enum so
/// it cannot drift.
pub fn qos_profiles() -> Vec<CompletionCandidate> {
    candidates(
        zenkey::qos::QosProfile::ALL
            .into_iter()
            .map(|p| p.name().to_string()),
    )
}

/// The `@blob` tier tokens — a closed vocabulary (RFC 07 §2), so this needs no
/// cache and cannot go stale.
pub fn blob_tiers() -> Vec<CompletionCandidate> {
    candidates(
        [
            zenkey::BlobTier::Artifact,
            zenkey::BlobTier::Tree,
            zenkey::BlobTier::Store,
        ]
        .into_iter()
        .map(|t| t.chunk().to_string()),
    )
}

/// Named contexts from the config file.
pub fn contexts() -> Vec<CompletionCandidate> {
    let Ok(config) = zenkey_fleet::context_store::load() else {
        return Vec::new();
    };
    candidates(config.contexts.keys().cloned())
}

/// Keys, completed from the *declared* keyspace: `v1/<origin>/<class>/…`.
///
/// Subject patterns reach the user through here rather than on their own: a
/// bare `disk/{mount}/used` is not something any argument takes, and offering
/// it as if it were would be a small lie. The `{var}` chunks are kept verbatim
/// so it is visible that a *shape* is being completed, not a key.
///
/// The origin position completes as `*` plus nothing else — the cache holds
/// registries, not a roster, and inventing origin ids from a slice would be
/// exactly the kind of guess RFC 09 §5.1 O3 forbids.
pub fn keys() -> Vec<CompletionCandidate> {
    keys_in(&cached())
}

fn keys_in(set: &SliceSet) -> Vec<CompletionCandidate> {
    let mut out: Vec<String> = Vec::new();
    for slice in set.slices() {
        // A service origin is verbatim and known; a host origin is not ours
        // to guess, so it stays a wildcard the user replaces.
        let origin = slice.service_origin.clone().unwrap_or_else(|| "*".into());
        for d in &slice.subjects {
            out.push(match &slice.service_origin {
                Some(_) => format!("v1/{origin}/{}/{}", d.class, d.path),
                None => format!("v1/{origin}/{}/{}/{}", d.class, slice.name, d.path),
            });
        }
    }
    candidates(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The closed vocabularies are the enums, not copies of them.
    #[test]
    fn closed_vocabularies_come_from_their_types() {
        let qos: Vec<String> = qos_profiles()
            .iter()
            .map(|c| c.get_value().to_string_lossy().to_string())
            .collect();
        assert_eq!(qos.len(), zenkey::qos::QosProfile::ALL.len());
        assert!(qos.contains(&"sampled".to_string()));

        let classes: Vec<String> = classes()
            .iter()
            .map(|c| c.get_value().to_string_lossy().to_string())
            .collect();
        assert_eq!(classes, ["events", "state", "telemetry"]);

        let tiers: Vec<String> = blob_tiers()
            .iter()
            .map(|c| c.get_value().to_string_lossy().to_string())
            .collect();
        assert_eq!(tiers, ["artifact", "store", "tree"]);
    }

    /// No cache, no candidates — and above all, no panic and no bus. This is
    /// the "degrades to static" half of #54's acceptance.
    ///
    /// Tested against an absent *directory* rather than by setting the config
    /// env var: that var is process-global, and a completion test that moved
    /// it would silently redirect whatever else was running in the same test
    /// binary.
    #[test]
    fn an_absent_cache_yields_nothing_rather_than_failing() {
        let empty = SliceSet::read_cache(std::path::Path::new("/nonexistent-zenctl-completion"));
        assert!(producers_in(&empty).is_empty());
        assert!(types_in(&empty).is_empty());
        assert!(procedures_in(&empty).is_empty());
        assert!(keys_in(&empty).is_empty());
        // …while the closed vocabularies still answer: they were never cached.
        assert!(!classes().is_empty());
    }

    /// And with a cache, the candidates are the cached names — including the
    /// key shapes, whose origin position stays a wildcard rather than a guess
    /// (RFC 09 §5.1 O3).
    #[test]
    fn a_cached_registry_supplies_names_and_key_shapes() {
        let dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixture-tests/registry");
        let set = SliceSet::from_dirs(&[dir]).expect("fixture registry");
        let names: Vec<String> = producers_in(&set)
            .iter()
            .map(|c| c.get_value().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"sysinfo".to_string()), "{names:?}");
        assert!(names.contains(&"catalog".to_string()), "the service too");

        let keys: Vec<String> = keys_in(&set)
            .iter()
            .map(|c| c.get_value().to_string_lossy().to_string())
            .collect();
        assert!(
            keys.iter().any(|k| k.starts_with("v1/*/state/sysinfo/")),
            "a host producer's origin stays `*`: {keys:?}"
        );
        assert!(
            keys.iter().any(|k| k.starts_with("v1/@catalog/state/")),
            "a service origin is verbatim and known: {keys:?}"
        );
    }
}

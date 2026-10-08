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

/// `zenctl completions <shell>` — the dynamic registration snippet, or the
/// static tree under `--static`.
///
/// Lived in the match arm until #209. The two forms are not alternatives so
/// much as a fallback: the dynamic completer asks the running binary (and so
/// can offer live keys, producers and types), the static one is a shell script
/// that knows only the tree — which is what a machine without this binary on
/// its `PATH` at completion time can use.
pub fn emit(shell: clap_complete::Shell, static_only: bool) -> Result<()> {
    use clap::CommandFactory as _;
    if static_only {
        clap_complete::aot::generate(
            shell,
            &mut crate::Cli::command(),
            "zenctl",
            &mut std::io::stdout(),
        );
        return Ok(());
    }
    registration(shell)
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

/// The `--context` already typed on the line being completed.
///
/// The cache is keyed by context name, so a completion that ignores `--context`
/// reads a *different* directory from the one the same command line wrote
/// (issue #197): `zenctl --context lab topic list` fills `…/cache/lab/slices`,
/// and completing `zenctl --context lab topic <TAB>` used to read
/// `…/cache/<current>/slices` — often empty, with no way for the user to tell.
///
/// The value has to come from here because the completion engine does not
/// offer it: `ValueCandidates::candidates(&self)` takes no arguments. What it
/// does offer is that the completer runs as a subprocess whose own argv *is*
/// the command line being completed, after a `--` sentinel
/// (`CompleteEnv::complete` reads `std::env::args_os()`).
fn context_on_line() -> Option<String> {
    context_in(std::env::args_os().filter_map(|a| a.into_string().ok()))
}

/// The pure half, so the parse is testable without a real command line.
fn context_in(args: impl IntoIterator<Item = String>) -> Option<String> {
    let words: Vec<String> = args.into_iter().collect();
    // Everything before the last `--` is the completer's own invocation.
    let start = words
        .iter()
        .rposition(|w| w == "--")
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut rest = words[start..].iter();
    while let Some(word) = rest.next() {
        let value = match word.strip_prefix("--context=") {
            Some(v) => Some(v.to_string()),
            None if word == "--context" => rest.next().cloned(),
            None => continue,
        };
        // `--context <TAB>` leaves an empty word: the user has not named a
        // context yet, so fall through to the ambient one rather than keying
        // the cache on "".
        return value.filter(|v| !v.is_empty());
    }
    None
}

/// The cached slices for the context this invocation would use.
///
/// Deliberately infallible: completion has no error channel that a user would
/// want to read mid-keystroke.
fn cached() -> SliceSet {
    let name = zenkey_explorer_config::active_name(context_on_line().as_deref());
    SliceSet::read_cache(&zenkey_explorer_config::cache_dir(name.as_deref()))
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
    // Read off the enum, like the QoS profiles below — a fourth hand-written
    // copy of a three-token closed set was one too many (#351).
    candidates(zenkey::Class::ALL.iter().map(|c| c.chunk().to_string()))
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
    let Ok(config) = zenkey_explorer_config::load() else {
        return Vec::new();
    };
    candidates(config.contexts.keys().cloned())
}

// ── zk2's names (#612, FJ4) ─────────────────────────────────────────────
//
// zk2 has no registry to cache: what a completion can offer is what a
// presence read last saw — service addresses and interfaces, per namespace
// — and which namespaces `namespace list` last saw. One small JSON file
// beside the slices, under the same three rules: never the bus, never a
// failure, never a claim. `cache clear` removes it with the slices.

/// The file, inside the context's cache directory. `SliceSet::read_cache`
/// reads only `*.toml`/`*.kdl` there, so it never mistakes this for a slice.
const ZK2_NAMES: &str = "zk2-names.json";

/// What presence reads last saw, per namespace.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Zk2Names {
    #[serde(default)]
    namespaces: std::collections::BTreeSet<String>,
    #[serde(default)]
    seen: std::collections::BTreeMap<String, Zk2Seen>,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Zk2Seen {
    #[serde(default)]
    services: std::collections::BTreeSet<String>,
    #[serde(default)]
    ifaces: std::collections::BTreeSet<String>,
}

fn zk2_path(context: Option<&str>) -> std::path::PathBuf {
    zenkey_explorer_config::cache_dir(zenkey_explorer_config::active_name(context).as_deref())
        .join(ZK2_NAMES)
}

fn read_names(path: &std::path::Path) -> Zk2Names {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Best-effort, like the slice cache: a cache that cannot be written must
/// not fail the command the user ran, so nothing here returns an error.
fn write_names(path: &std::path::Path, names: &Zk2Names) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(names) {
        let _ = std::fs::write(path, bytes);
    }
}

/// Remember what one presence read in `namespace` saw. A read of the whole
/// namespace (`whole`) replaces what was remembered there; a narrower one
/// adds to it. Nothing seen writes nothing: one empty read must not blank a
/// good cache.
pub(crate) fn remember_presence(
    context: Option<&str>,
    namespace: &str,
    whole: bool,
    services: impl IntoIterator<Item = String>,
    ifaces: impl IntoIterator<Item = String>,
) {
    remember_into(&zk2_path(context), namespace, whole, services, ifaces);
}

/// [`remember_presence`] against an explicit file: the pure half a test can
/// drive without touching the user's cache directory.
fn remember_into(
    path: &std::path::Path,
    namespace: &str,
    whole: bool,
    services: impl IntoIterator<Item = String>,
    ifaces: impl IntoIterator<Item = String>,
) {
    let services: std::collections::BTreeSet<String> = services.into_iter().collect();
    let ifaces: std::collections::BTreeSet<String> = ifaces.into_iter().collect();
    if services.is_empty() && ifaces.is_empty() {
        return;
    }
    let mut names = read_names(path);
    let seen = names.seen.entry(namespace.to_owned()).or_default();
    if whole {
        *seen = Zk2Seen::default();
    }
    seen.services.extend(services);
    seen.ifaces.extend(ifaces);
    write_names(path, &names);
}

/// Remember the namespaces one `namespace list` saw (replacing the last).
pub(crate) fn remember_namespaces(
    context: Option<&str>,
    namespaces: impl IntoIterator<Item = String>,
) {
    let namespaces: std::collections::BTreeSet<String> = namespaces.into_iter().collect();
    if namespaces.is_empty() {
        return;
    }
    let path = zk2_path(context);
    let mut names = read_names(&path);
    names.namespaces = namespaces;
    write_names(&path, &names);
}

/// The value of `--namespace` (or `--base`) on the line being completed.
fn namespace_in(args: impl IntoIterator<Item = String>) -> Option<String> {
    let words: Vec<String> = args.into_iter().collect();
    let start = words
        .iter()
        .rposition(|w| w == "--")
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut rest = words[start..].iter();
    let mut found = None;
    while let Some(word) = rest.next() {
        let value =
            ["--namespace", "--base"]
                .iter()
                .find_map(|flag| match word.strip_prefix(flag) {
                    Some("") => Some(rest.clone().next().cloned()),
                    Some(v) => v.strip_prefix('=').map(|v| Some(v.to_owned())),
                    None => None,
                });
        if let Some(v) = value {
            found = v;
        }
    }
    found
}

/// Which namespace the line being completed reads: its `--namespace`,
/// else `ZENCTL_BASE`, else the active context's base, else the bus root —
/// the flag's own ladder, read without failing.
fn namespace_on_line(context: Option<&str>) -> String {
    let args: Vec<String> = std::env::args_os()
        .filter_map(|a| a.into_string().ok())
        .collect();
    namespace_in(args)
        .or_else(|| std::env::var("ZENCTL_BASE").ok())
        .or_else(|| {
            zenkey_explorer_config::active(context)
                .ok()
                .flatten()
                .and_then(|c| c.base)
        })
        .unwrap_or_default()
}

/// Service addresses a presence read last saw in the line's namespace.
pub fn services() -> Vec<CompletionCandidate> {
    let context = context_on_line();
    let names = read_names(&zk2_path(context.as_deref()));
    let ns = namespace_on_line(context.as_deref());
    candidates(
        names
            .seen
            .get(&ns)
            .map(|s| s.services.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default(),
    )
}

/// Interfaces a presence read last saw in the line's namespace.
pub fn ifaces() -> Vec<CompletionCandidate> {
    let context = context_on_line();
    let names = read_names(&zk2_path(context.as_deref()));
    let ns = namespace_on_line(context.as_deref());
    candidates(
        names
            .seen
            .get(&ns)
            .map(|s| s.ifaces.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default(),
    )
}

/// Namespaces `namespace list` last saw, the bus root left out (it is
/// spelled `''`, which a shell completes poorly and nobody needs offered).
pub fn namespaces() -> Vec<CompletionCandidate> {
    let names = read_names(&zk2_path(context_on_line().as_deref()));
    candidates(names.namespaces.into_iter().filter(|n| !n.is_empty()))
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
        let origin = slice
            .service_origin
            .as_ref()
            .map(zenkey::Declared::token)
            .unwrap_or("*");
        for d in &slice.subjects {
            out.push(match &slice.service_origin {
                Some(_) => format!("v1/{origin}/{}/{}", d.class.token(), d.path),
                None => {
                    format!("v1/{origin}/{}/{}/{}", d.class.token(), slice.name, d.path)
                }
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

    /// The cache is keyed by context, so the completion must read the key the
    /// same command line *wrote* (#197). The completer sees the line as its
    /// own argv, after a `--` sentinel.
    #[test]
    fn the_completion_reads_the_context_the_command_line_names() {
        let line = |w: &[&str]| {
            let mut v = vec!["zenctl".to_string(), "--".to_string()];
            v.extend(w.iter().map(|s| s.to_string()));
            v
        };

        // Both spellings, wherever they sit on the line.
        assert_eq!(
            context_in(line(&["zenctl", "--context", "lab", "topic", ""])),
            Some("lab".into())
        );
        assert_eq!(
            context_in(line(&["zenctl", "--context=lab", "topic", ""])),
            Some("lab".into())
        );
        assert_eq!(
            context_in(line(&["zenctl", "topic", "list", "--context", "prod", ""])),
            Some("prod".into())
        );

        // No context named: the ambient one (env, then the `current` pointer)
        // still applies, which is what `active_name(None)` resolves.
        assert_eq!(context_in(line(&["zenctl", "topic", ""])), None);
        // `--context <TAB>`: nothing named yet. Keying the cache on "" would
        // read a directory nothing ever writes.
        assert_eq!(context_in(line(&["zenctl", "--context", ""])), None);
        // The completer's own argv before the sentinel is not the user's line.
        assert_eq!(
            context_in(vec![
                "/usr/bin/zenctl".to_string(),
                "--context".to_string(),
                "notmine".to_string(),
                "--".to_string(),
                "zenctl".to_string(),
                "topic".to_string(),
                String::new(),
            ]),
            None
        );
    }

    /// zk2's names (FJ4): the line's `--namespace` (or its alias `--base`)
    /// picks which namespace's names are offered; a read of the whole
    /// namespace replaces what it remembered, a narrower read adds to it,
    /// and an empty read writes nothing, so one bad moment cannot blank a
    /// good cache.
    #[test]
    fn zk2_names_are_remembered_per_namespace() {
        let line = |w: &[&str]| {
            let mut v = vec!["zenctl".to_string(), "--".to_string()];
            v.extend(w.iter().map(|s| s.to_string()));
            v
        };
        assert_eq!(
            namespace_in(line(&[
                "zenctl",
                "service",
                "show",
                "--namespace",
                "prod",
                ""
            ])),
            Some("prod".into())
        );
        assert_eq!(
            namespace_in(line(&["zenctl", "iface", "show", "--base=site/a", ""])),
            Some("site/a".into())
        );
        assert_eq!(
            namespace_in(line(&["zenctl", "--namespacex", "y", "service", ""])),
            None
        );
        assert_eq!(namespace_in(line(&["zenctl", "service", "show", ""])), None);

        let path = std::env::temp_dir().join(format!(
            "zenctl-zk2-names-{}-{}.json",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_file(&path);
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        remember_into(
            &path,
            "prod",
            true,
            s(&["host-a/tc", "host-b/tc"]),
            s(&["tc.netif.v1"]),
        );
        remember_into(&path, "prod", false, s(&["ws-01/gui"]), s(&[]));
        remember_into(&path, "", true, s(&["root/svc"]), s(&[]));
        remember_into(&path, "prod", true, s(&[]), s(&[]));
        let names = read_names(&path);
        let prod = &names.seen["prod"];
        assert_eq!(
            prod.services.iter().map(String::as_str).collect::<Vec<_>>(),
            ["host-a/tc", "host-b/tc", "ws-01/gui"],
            "a narrower read adds; an empty one writes nothing"
        );
        assert_eq!(names.seen[""].services.len(), 1, "namespaces stay apart");
        remember_into(&path, "prod", true, s(&["host-c/tc"]), s(&[]));
        assert_eq!(
            read_names(&path).seen["prod"].services.len(),
            1,
            "a whole read replaces"
        );
        let _ = std::fs::remove_file(&path);
    }
}

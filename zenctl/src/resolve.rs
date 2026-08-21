//! The resolution ladders, as pure functions (#209).
//!
//! Every knob this tool takes is resolved the same way — **flag > env >
//! active context > default** — and each rung was written out again wherever
//! it was needed. `session()` and `session_reporting()` were verbatim copies
//! of one four-rung climb; `Scout`, which has no `BusArgs` to hang methods on,
//! had a third copy of two of them; `cmd/watch.rs` re-derived the slice-source
//! decision by hand.
//!
//! ## Scalars, not `&BusArgs`
//!
//! A ladder that demands the whole argument struct cannot serve the caller
//! that does not have one — and `Scout` is the standing proof of what happens
//! then: it gets copy-pasted instead of called. So these take the flag value
//! and the stored context, and `BusArgs` adapts.
//!
//! ## The second parameter is the testability
//!
//! `stored: Option<&StoredContext>` passed in — rather than read from
//! `~/.config` inside — removes the config file, the process-global cache and
//! the `exit(2)` from every test at once. Before this, the crate's only unit
//! test of a `BusArgs` accessor passed *because it never let the ladder reach
//! its second rung* (`cmd/mod.rs` sets `base: Some("zs")`): with `base: None`
//! it would have read the developer's real config.
//!
//! Nothing here opens a file, a session, or a process. The engine legislated
//! this for its own half already — `zenkey_fleet::context_store` opens with
//! "Everything here is pure and fallible … zenctl turns the `Err` into its own
//! exit code at its own edge" — and this is zenctl keeping that bargain.

use std::path::{Path, PathBuf};
use std::time::Duration;

use zenkey_fleet::context_store::StoredContext;

/// The default reply/watch window, in seconds, when neither flag nor context
/// names one.
pub const DEFAULT_TIMEOUT_S: u64 = 5;

/// The deployment base: flag (or `ZENCTL_BASE`, which clap folds into the
/// flag) > active context > empty.
///
/// Empty is a **deployment**, not an absence: the base-less bus root, whose
/// wire keys start at `v1/` — the RFC v1.6 default. `--base ''` therefore
/// resolves to the same place as no base at all, deliberately, and must not
/// fall through to the context.
pub fn base<'a>(flag: Option<&'a str>, stored: Option<&'a StoredContext>) -> &'a str {
    match flag {
        Some(b) => b,
        None => stored.and_then(|c| c.base.as_deref()).unwrap_or(""),
    }
}

/// Endpoints: a non-empty flag list **replaces** the context's, never merges
/// with it.
///
/// `-c tcp/localhost:7447` on a context that names a production router has to
/// mean "talk to this one", not "talk to both" — an explorer that quietly
/// joins a mesh the user did not name is the same failure `--scouting` is off
/// by default to avoid.
pub fn endpoints(flag: &[String], stored: Option<&[String]>) -> Vec<String> {
    if flag.is_empty() {
        stored.map(<[String]>::to_vec).unwrap_or_default()
    } else {
        flag.to_vec()
    }
}

/// Multicast scouting: `Some(true)` when the flag is given, otherwise the
/// context's own choice, otherwise **unstated**.
///
/// Three states, not two. A `--zenoh-config` file may set scouting itself, and
/// an absent flag has to leave that alone (#122) — folding "not given" into
/// `Some(false)` would have the CLI silently overrule a file the user wrote.
/// This is why the flag is a bare `bool` and the answer is an `Option`.
pub fn scouting(flag: bool, stored: Option<&StoredContext>) -> Option<bool> {
    if flag {
        Some(true)
    } else {
        stored.and_then(|c| c.scouting)
    }
}

/// The reply/watch window: flag > context > [`DEFAULT_TIMEOUT_S`].
///
/// **No env rung**, unlike `base` — and that asymmetry is deliberate rather
/// than an oversight: `ZENCTL_BASE` names *which deployment you are looking
/// at*, which a shell session sensibly fixes once; a timeout is per-question.
pub fn timeout(flag: Option<u64>, stored: Option<&StoredContext>) -> Duration {
    Duration::from_secs(
        flag.or_else(|| stored.and_then(|c| c.timeout))
            .unwrap_or(DEFAULT_TIMEOUT_S),
    )
}

/// Registry directories: a non-empty flag list replaces the context's, on the
/// same rule as [`endpoints`].
pub fn registry_dirs(flag: &[PathBuf], stored: Option<&StoredContext>) -> Vec<PathBuf> {
    if flag.is_empty() {
        stored.map(|c| c.registry.clone()).unwrap_or_default()
    } else {
        flag.to_vec()
    }
}

/// The zenoh JSON5 config file (#122): flag (or `ZENCTL_ZENOH_CONFIG`) >
/// context > none.
pub fn zenoh_config(flag: Option<&Path>, stored: Option<&StoredContext>) -> Option<PathBuf> {
    flag.map(Path::to_path_buf)
        .or_else(|| stored.and_then(|c| c.zenoh_config.clone()))
}

/// Everything `zenkey_fleet::open_reporting` needs, climbed once.
///
/// One struct because the four rungs were always taken together, and taking
/// them together in two places is how `session()` and `session_reporting()`
/// came to be verbatim copies of each other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transport {
    pub file: Option<PathBuf>,
    pub connect: Vec<String>,
    pub listen: Vec<String>,
    pub scouting: Option<bool>,
}

/// The transport ladder: what to open, from flags and the active context.
pub fn transport(
    zenoh_config_flag: Option<&Path>,
    connect_flag: &[String],
    listen_flag: &[String],
    scouting_flag: bool,
    stored: Option<&StoredContext>,
) -> Transport {
    Transport {
        file: zenoh_config(zenoh_config_flag, stored),
        connect: endpoints(connect_flag, stored.map(|c| c.connect.as_slice())),
        listen: endpoints(listen_flag, stored.map(|c| c.listen.as_slice())),
        scouting: scouting(scouting_flag, stored),
    }
}

/// Where registry slices come from for this invocation.
///
/// `--registry` dirs, when given, do not *replace* the bus — they union with
/// it, served winning per producer (RFC 08 §6.1, issue #43). What they change
/// is whether a bus that will not come up is fatal: a question the dirs can
/// answer on their own must not die with the transport (#196).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SliceSource {
    /// Live introspection only — the bus is the whole answer.
    Bus,
    /// The union of the bus and these directories.
    Union(Vec<PathBuf>),
}

/// Which source this invocation's slices come from.
pub fn slice_source(dirs: Vec<PathBuf>) -> SliceSource {
    if dirs.is_empty() {
        SliceSource::Bus
    } else {
        SliceSource::Union(dirs)
    }
}

/// The sentences the slice ladder says about its own coverage.
///
/// They are notes, not log lines: each one is a claim about **what was
/// asked** (RFC 09 §5.1 O4/O5) or about a zero that is not a verdict
/// (RFC 05 §3.1). Built here, printed by the caller, so what they say is
/// testable without a bus.
pub mod notes {
    use std::path::Path;

    use crate::render::Note;

    /// The transport would not come up, and the dirs answered instead.
    ///
    /// Pinned byte-for-byte by `tests/cmd/session-transport-fallback.trycmd`,
    /// including the trailing period after the citation — which is why the
    /// citation is part of the text here rather than a `cite`.
    pub fn registry_only(error: &str, dirs: &[impl AsRef<Path>]) -> Note {
        Note::coverage(format!(
            "no session ({error}); answering from --registry only: {}.\n\
             That is what this checkout declares, not what the fleet \
             serves — `zenctl doctor --registry <dir>` compares them \
             when the bus is reachable (RFC 05 §3.1).",
            dirs.iter()
                .map(|d| d.as_ref().display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    /// The bus and a directory declare the same producer differently.
    ///
    /// Reported, never silently overwritten: the union has a winner, and the
    /// loser having existed is a fact about the deployment.
    pub fn disagreement(
        producer: &str,
        bus_version: &str,
        dirs_version: &str,
        shape_differs: bool,
    ) -> Note {
        Note::caveat(format!(
            "registry disagreement: {producer} — bus serves v{bus_version}, \
             dirs carry v{dirs_version}{} (served wins; `zenctl doctor \
             --registry <dir>` details the drift)",
            if shape_differs { ", shapes differ" } else { "" }
        ))
    }

    /// Nobody answered the introspect sweep.
    ///
    /// An empty set is not "this deployment has no registry"; it is "nothing
    /// answered", and the note has to say which, with the two questions that
    /// tell them apart.
    pub fn no_slices(base: &str) -> Note {
        Note::silence(format!(
            "no introspect slices on base {base:?} — an empty set is not a verdict (RFC 05 §3.1); \
             `zenctl node list --base {base:?}` says who is actually up.\n\
             (offline alternative: --registry <dir> with the app's registry TOMLs)"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> StoredContext {
        StoredContext {
            base: Some("prod".into()),
            connect: vec!["tcp/router:7447".into()],
            listen: vec!["tcp/0.0.0.0:0".into()],
            registry: vec![PathBuf::from("/ctx/registry")],
            scouting: Some(false),
            zenoh_config: Some(PathBuf::from("/ctx/zenoh.json5")),
            timeout: Some(30),
        }
    }

    /// An explicitly empty base is a deployment — the bus root — and must not
    /// fall through to the context. `--base ''` is how you look at a base-less
    /// fleet from a shell whose context names a based one.
    #[test]
    fn an_empty_base_is_a_deployment_not_an_absence() {
        let c = ctx();
        assert_eq!(base(Some(""), Some(&c)), "");
        assert_eq!(base(Some("lab"), Some(&c)), "lab");
        assert_eq!(base(None, Some(&c)), "prod");
        assert_eq!(base(None, None), "", "no context, no base: the bus root");
    }

    /// A non-empty `--connect` replaces the context's endpoints. Merging would
    /// mean `-c tcp/localhost:7447` still talks to production.
    #[test]
    fn endpoints_replace_rather_than_merge() {
        let c = ctx();
        assert_eq!(
            endpoints(&["tcp/localhost:7447".to_string()], Some(&c.connect)),
            vec!["tcp/localhost:7447".to_string()]
        );
        assert_eq!(endpoints(&[], Some(&c.connect)), c.connect);
        assert!(endpoints(&[], None).is_empty());
    }

    /// #122: an unstated `--scouting` leaves a config file's own choice alone.
    /// Three states, and folding the middle one into `false` would have the
    /// CLI overrule a file the user wrote.
    #[test]
    fn an_unstated_scouting_flag_stays_unstated() {
        let mut c = ctx();
        assert_eq!(scouting(false, Some(&c)), Some(false));
        assert_eq!(scouting(true, Some(&c)), Some(true));
        c.scouting = None;
        assert_eq!(
            scouting(false, Some(&c)),
            None,
            "not given is not the same as off — the config file decides"
        );
        assert_eq!(scouting(false, None), None);
    }

    /// The timeout ladder has no env rung, unlike `base`. Deliberate: a base
    /// is a shell-session fact, a timeout is per-question.
    #[test]
    fn the_timeout_ladder_has_two_rungs_and_a_default() {
        let c = ctx();
        assert_eq!(timeout(Some(1), Some(&c)), Duration::from_secs(1));
        assert_eq!(timeout(None, Some(&c)), Duration::from_secs(30));
        assert_eq!(
            timeout(None, None),
            Duration::from_secs(DEFAULT_TIMEOUT_S),
            "5s, and the flag's help text says so"
        );
    }

    /// The whole transport climb, in the one call `session()` and
    /// `session_reporting()` used to make twice, identically.
    #[test]
    fn the_transport_ladder_climbs_all_four_rungs_together() {
        let c = ctx();
        assert_eq!(
            transport(None, &[], &[], false, Some(&c)),
            Transport {
                file: Some(PathBuf::from("/ctx/zenoh.json5")),
                connect: vec!["tcp/router:7447".into()],
                listen: vec!["tcp/0.0.0.0:0".into()],
                scouting: Some(false),
            }
        );
        let flags = transport(
            Some(Path::new("/flag.json5")),
            &["tcp/localhost:7447".to_string()],
            &[],
            true,
            Some(&c),
        );
        assert_eq!(flags.file, Some(PathBuf::from("/flag.json5")));
        assert_eq!(flags.connect, vec!["tcp/localhost:7447".to_string()]);
        assert_eq!(
            flags.listen, c.listen,
            "an untouched rung still comes from the context"
        );
        assert_eq!(flags.scouting, Some(true));
        assert_eq!(
            transport(None, &[], &[], false, None),
            Transport {
                file: None,
                connect: vec![],
                listen: vec![],
                scouting: None,
            }
        );
    }

    /// Registry dirs climb the same replace-not-merge ladder as endpoints, and
    /// the source decision reads off the result — one place, so `watch` cannot
    /// re-derive it differently.
    #[test]
    fn the_slice_source_follows_the_dirs() {
        let c = ctx();
        assert_eq!(registry_dirs(&[], Some(&c)), c.registry);
        assert_eq!(
            registry_dirs(&[PathBuf::from("/flag")], Some(&c)),
            vec![PathBuf::from("/flag")]
        );
        assert_eq!(slice_source(vec![]), SliceSource::Bus);
        assert_eq!(
            slice_source(registry_dirs(&[], Some(&c))),
            SliceSource::Union(c.registry.clone()),
            "a context's dirs select the union just as the flag does"
        );
        assert_eq!(slice_source(registry_dirs(&[], None)), SliceSource::Bus);
    }

    /// The registry-only note is pinned byte-for-byte by the corpus. Asserting
    /// its shape here means a copy-edit fails a unit test before it fails a
    /// trycmd file nobody was looking at.
    #[test]
    fn the_registry_only_note_names_the_dirs_and_the_bound() {
        let n = notes::registry_only("port in use", &[Path::new("/a"), Path::new("/b")]);
        let line = n.to_line();
        assert!(line.starts_with("no session (port in use); "), "{line}");
        assert!(line.contains("--registry only: /a, /b."), "{line}");
        assert!(
            line.contains("what this checkout declares, not what the fleet serves"),
            "the claim is about coverage, not about the fleet: {line}"
        );
    }

    /// A zero from the bus is a zero, and says so with the question that
    /// distinguishes "nothing answered" from "nothing is declared".
    #[test]
    fn an_empty_sweep_is_never_a_verdict() {
        let line = notes::no_slices("zensight").to_line();
        assert!(line.contains("not a verdict (RFC 05 §3.1)"), "{line}");
        assert!(line.contains("node list --base"), "{line}");
        assert!(
            line.contains("--registry <dir>"),
            "the offline alternative rides along: {line}"
        );
    }

    /// A disagreement is reported with both versions, and the shape clause
    /// appears only when shapes actually differ.
    #[test]
    fn a_disagreement_names_both_versions() {
        let with = notes::disagreement("sysinfo", "2", "1", true).to_line();
        assert!(
            with.contains("bus serves v2, dirs carry v1, shapes differ"),
            "{with}"
        );
        let without = notes::disagreement("sysinfo", "2", "1", false).to_line();
        assert!(without.contains("dirs carry v1 (served wins"), "{without}");
    }
}

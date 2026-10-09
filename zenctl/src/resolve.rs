//! The resolution ladders, as pure functions (#209).
//!
//! Every knob this tool takes is resolved the same way — **flag > env >
//! active context > default** — and each rung was written out again wherever
//! it was needed. `session()` and `session_reporting()` were verbatim copies
//! of one four-rung climb, and `Scout`, which has no argument struct to hang
//! methods on, had a third copy of two of them.
//!
//! ## Scalars, not argument structs
//!
//! A ladder that demands the whole argument struct cannot serve the caller
//! that does not have one — and `Scout` is the standing proof of what happens
//! then: it gets copy-pasted instead of called. So these take the flag value
//! and the stored context, and `crate::bus` adapts. The `--registry` ladder
//! and the slice-source decision left with the v1 registry (#612, FJ9).
//!
//! ## The second parameter is the testability
//!
//! `stored: Option<&StoredContext>` passed in — rather than read from
//! `~/.config` inside — removes the config file, the process-global cache and
//! the `exit(2)` from every test at once. Before this, the crate's only unit
//! test of a bus accessor passed *because it never let the ladder reach its
//! second rung*: with no base it would have read the developer's real
//! config.
//!
//! Nothing here opens a file, a session, or a process. The engine legislated
//! this for its own half already — `zenkey_explorer_config` opens with
//! "Everything here is pure and fallible … zenctl turns the `Err` into its own
//! exit code at its own edge" — and this is zenctl keeping that bargain.

use std::path::{Path, PathBuf};
use std::time::Duration;

use zenkey_explorer_config::StoredContext;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> StoredContext {
        StoredContext {
            base: Some("prod".into()),
            connect: vec!["tcp/router:7447".into()],
            listen: vec!["tcp/0.0.0.0:0".into()],
            scouting: Some(false),
            zenoh_config: Some(PathBuf::from("/ctx/zenoh.json5")),
            timeout: Some(30),
            ..StoredContext::default()
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
}

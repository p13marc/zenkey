//! The command line, resolved into plain owned settings.
//!
//! Modelled on `zenctl`'s `BusArgs`, with one deliberate difference: resolution
//! here is **fallible and pure**. `BusArgs::stored()` memoises into a
//! `static OnceLock` and calls `std::process::exit(2)` on a bad context — fine
//! for a CLI that is about to run one command and die, unusable in a GUI, where
//! a bad input must surface as a banner and leave the window open.

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

use crate::scope::ScopePreset;

/// The zengui command line.
#[derive(Debug, Clone, Parser)]
#[command(name = "zengui", about, version)]
pub struct Cli {
    /// The deployment base — the first chunk(s) of every key on the wire.
    ///
    /// Applications set this as their Zenoh session `namespace` and never spell
    /// it. An explorer deliberately does not: it runs un-namespaced so it sees
    /// the wire as it really is, including traffic from outside the deployment
    /// (RFC 09 §5) — which is what lets it spot a leak. So it has to be told.
    ///
    /// Omitted, or `--base ""`, means the base-less bus root: the RFC v1.6
    /// default, whose wire keys start at `v1/`. Bases actually in use are
    /// discovered at runtime, so this flag is a shortcut, not a requirement.
    #[arg(long, env = "ZENGUI_BASE")]
    pub base: Option<String>,

    /// Use a named context from the shared explorer config
    /// (`~/.config/zenkey-explorer/config.toml`, with a read-fallback to the
    /// legacy zenctl location). Flags always override context values.
    /// Default: the file's `current` pointer, or the
    /// `ZENKEY_EXPLORER_CONTEXT`/`ZENCTL_CONTEXT` env vars.
    #[arg(long, value_name = "NAME")]
    pub context: Option<String>,

    /// Endpoint to connect to, repeatable (e.g. `tcp/127.0.0.1:7447`).
    #[arg(long, short = 'c', value_name = "ENDPOINT")]
    pub connect: Vec<String>,

    /// Endpoint to listen on, repeatable.
    #[arg(long, short = 'l', value_name = "ENDPOINT")]
    pub listen: Vec<String>,

    /// Enable multicast scouting.
    ///
    /// Off by default, and deliberately so: an explorer that multicast-scouts
    /// joins whatever mesh it can reach, which is how a throwaway session ends
    /// up in a live fleet. Note RFC 09 §0.1 — this is the *multicast* half
    /// only; gossip is a separate switch.
    #[arg(long)]
    pub scouting: bool,

    /// Directory of registry TOMLs, repeatable. When given, registry slices are
    /// read from disk instead of from the bus (RFC 08 §6 introspection).
    #[arg(long, value_name = "DIR")]
    pub registry: Vec<PathBuf>,

    /// Query timeout in seconds (default 5; a context may override the
    /// default, an explicit flag overrides the context).
    #[arg(long)]
    pub timeout: Option<u64>,

    /// What to watch.
    #[arg(long, value_enum, default_value_t = ScopePreset::Everything)]
    pub scope: ScopePreset,

    /// A key expression to watch, repeatable. Implies `--scope custom`.
    #[arg(long, value_name = "KEYEXPR")]
    pub selector: Vec<String>,

    /// How many echo lines to retain.
    #[arg(long, default_value_t = 2000)]
    pub echo_lines: usize,

    /// How many distinct keys to keep statistics for.
    ///
    /// Least-recently-seen keys are retired past this, and the retirements are
    /// counted and displayed — a long-running observer is bounded, and says so
    /// (RFC 09 §5.1 O6). Raise it on a bus with a very wide key population.
    #[arg(long, default_value_t = zenkey_fleet::stats::DEFAULT_MAX_KEYS)]
    pub max_keys: usize,
}

/// Resolved, owned settings. No `Option` gymnastics past this point.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Settings {
    /// The deployment base. Empty is legal and is the default (RFC v1.6).
    pub base: String,
    pub connect: Vec<String>,
    pub listen: Vec<String>,
    pub scouting: bool,
    pub registry: Vec<PathBuf>,
    pub timeout_secs: u64,
    pub scope: ScopePreset,
    pub selectors: Vec<String>,
    pub echo_lines: usize,
    pub max_keys: usize,
}

impl Cli {
    /// Resolve into settings, or explain why not: loads the active shared
    /// context (issue #35) and layers flags over it.
    pub fn settings(self) -> anyhow::Result<Settings> {
        let context = zenkey_fleet::context_store::active(self.context.as_deref())?;
        self.settings_with(context)
    }

    /// The pure half: `context` supplies defaults, flags override. Split from
    /// [`Cli::settings`] so tests never touch the user's real config file.
    pub fn settings_with(
        self,
        context: Option<zenkey_fleet::StoredContext>,
    ) -> anyhow::Result<Settings> {
        let context = context.unwrap_or_default();
        let scope = if self.selector.is_empty() {
            self.scope
        } else {
            ScopePreset::Custom
        };
        if scope == ScopePreset::Custom && self.selector.is_empty() {
            anyhow::bail!("--scope custom needs at least one --selector");
        }
        for sel in &self.selector {
            crate::scope::validate_selector(sel)?;
        }
        if self.echo_lines == 0 {
            anyhow::bail!("--echo-lines must be at least 1");
        }
        if self.max_keys == 0 {
            anyhow::bail!("--max-keys must be at least 1");
        }
        Ok(Settings {
            // Flag/env > context > "" (the bus root). Unset and `--base ""`
            // both resolve to the empty base — a real deployment, not a blank.
            base: self.base.or(context.base).unwrap_or_default(),
            connect: if self.connect.is_empty() {
                context.connect
            } else {
                self.connect
            },
            listen: if self.listen.is_empty() {
                context.listen
            } else {
                self.listen
            },
            scouting: self.scouting || context.scouting.unwrap_or(false),
            registry: if self.registry.is_empty() {
                context.registry
            } else {
                self.registry
            },
            timeout_secs: self.timeout.or(context.timeout).unwrap_or(5),
            scope,
            selectors: self.selector,
            echo_lines: self.echo_lines,
            max_keys: self.max_keys,
        })
    }
}

impl Settings {
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }

    /// How the base should be shown to a human.
    ///
    /// The empty base is a real, named deployment — the default one — not an
    /// unset field. RFC 09 §5: "an explorer that cannot see and name it is
    /// blind to the common case."
    pub fn base_label(&self) -> &str {
        if self.base.is_empty() {
            "(empty — keys start at v1/)"
        } else {
            &self.base
        }
    }

    /// Whether this session can observe anything at all.
    ///
    /// With no endpoints and no scouting, zenoh opens a session that reaches
    /// nothing. The window would then show an empty tree, which is
    /// indistinguishable from a quiet bus — the false verdict of RFC 05 §3.1.
    /// The UI turns this into a banner rather than letting the user infer.
    pub fn is_unreachable(&self) -> bool {
        self.connect.is_empty() && self.listen.is_empty() && !self.scouting
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> anyhow::Result<Settings> {
        // settings_with(None), not settings(): tests must never read the
        // user's real config file.
        Cli::try_parse_from(std::iter::once("zengui").chain(args.iter().copied()))?
            .settings_with(None)
    }

    #[test]
    fn unset_base_and_explicit_empty_base_agree() {
        assert_eq!(parse(&[]).unwrap().base, "");
        assert_eq!(parse(&["--base", ""]).unwrap().base, "");
        assert_eq!(parse(&["--base", "zensight"]).unwrap().base, "zensight");
    }

    /// The empty base is a deployment, not an unset field.
    #[test]
    fn the_empty_base_is_named_not_blank() {
        let s = parse(&[]).unwrap();
        assert!(s.base_label().contains("empty"));
        assert_eq!(
            parse(&["--base", "zensight"]).unwrap().base_label(),
            "zensight"
        );
    }

    #[test]
    fn defaults_match_the_documented_ones() {
        let s = parse(&[]).unwrap();
        assert_eq!(s.timeout(), Duration::from_secs(5));
        assert!(!s.scouting, "scouting is opt-in (RFC 09 §0.1)");
        assert_eq!(s.scope, ScopePreset::Everything);
        assert_eq!(s.echo_lines, 2000);
        assert_eq!(s.max_keys, zenkey_fleet::stats::DEFAULT_MAX_KEYS);
    }

    /// The key table is bounded (RFC 09 §5.1 O6); a zero bound is rejected at
    /// the boundary rather than silently clamped somewhere downstream.
    #[test]
    fn the_key_bound_is_configurable_and_must_be_positive() {
        assert_eq!(parse(&["--max-keys", "10"]).unwrap().max_keys, 10);
        assert!(parse(&["--max-keys", "0"]).is_err());
    }

    #[test]
    fn selectors_imply_custom_scope() {
        let s = parse(&["--selector", "demo/**"]).unwrap();
        assert_eq!(s.scope, ScopePreset::Custom);
        assert_eq!(s.selectors, ["demo/**"]);
    }

    #[test]
    fn custom_scope_without_selectors_is_an_error() {
        assert!(parse(&["--scope", "custom"]).is_err());
    }

    #[test]
    fn a_bad_selector_is_rejected_at_the_boundary() {
        assert!(parse(&["--selector", "demo/$*/x"]).is_err());
    }

    /// A session with nothing to reach must be reported, not silently empty.
    #[test]
    fn a_session_that_reaches_nothing_is_detectable() {
        assert!(parse(&[]).unwrap().is_unreachable());
        assert!(
            !parse(&["-c", "tcp/127.0.0.1:7447"])
                .unwrap()
                .is_unreachable()
        );
        assert!(!parse(&["--scouting"]).unwrap().is_unreachable());
    }

    /// Context supplies defaults; flags override (issue #35).
    #[test]
    fn context_supplies_defaults_and_flags_override() {
        let ctx = zenkey_fleet::StoredContext {
            base: Some("zensight".into()),
            connect: vec!["tcp/10.0.0.1:7447".into()],
            listen: vec![],
            registry: vec![],
            scouting: Some(true),
            timeout: Some(9),
        };
        let cli = |args: &[&str]| {
            Cli::try_parse_from(std::iter::once("zengui").chain(args.iter().copied())).unwrap()
        };
        // No flags: the context wins everywhere.
        let s = cli(&[]).settings_with(Some(ctx.clone())).unwrap();
        assert_eq!(s.base, "zensight");
        assert_eq!(s.connect, ["tcp/10.0.0.1:7447"]);
        assert!(s.scouting);
        assert_eq!(s.timeout_secs, 9);
        // Flags override field-by-field, including an explicit empty base.
        let s = cli(&["--base", "", "-c", "tcp/127.0.0.1:7447", "--timeout", "2"])
            .settings_with(Some(ctx))
            .unwrap();
        assert_eq!(s.base, "");
        assert_eq!(s.connect, ["tcp/127.0.0.1:7447"]);
        assert_eq!(s.timeout_secs, 2);
    }
}

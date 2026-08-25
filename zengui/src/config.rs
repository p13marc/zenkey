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

    /// Zenoh JSON5 config file (#122): the passthrough that reaches a
    /// secured bus (TLS, QUIC with certs, usrpwd, …). Loaded as the base
    /// layer; --connect/--listen/--scouting apply on top when given
    /// (flag > env > context > file). A file that sets a session namespace
    /// is refused — explorers run un-namespaced (RFC 09 §5).
    #[arg(long, value_name = "FILE", env = "ZENGUI_ZENOH_CONFIG")]
    pub zenoh_config: Option<PathBuf>,

    /// What to watch. Defaults to the scope this window had last time, then
    /// to `everything` — a flag, when given, always wins (issue #189).
    #[arg(long, value_enum)]
    pub scope: Option<ScopePreset>,

    /// A key expression to watch, repeatable. Implies `--scope custom`.
    #[arg(long, value_name = "KEYEXPR")]
    pub selector: Vec<String>,

    /// Start observing the scope immediately (the pre-lazy behavior).
    ///
    /// Without it zengui starts from the declared skeleton with zero
    /// data-plane subscriptions; observation is opt-in per subtree or via
    /// the "observe scope" toggle (issue #85).
    #[arg(long)]
    pub eager: bool,

    /// How many echo lines to retain (default 2000; the Settings overlay
    /// changes it live, and remembers the change — a flag beats the memory
    /// for this launch without becoming it).
    #[arg(long)]
    pub echo_lines: Option<usize>,

    /// How many samples of the selected key's history to retain (issue #63;
    /// default 200, remembered like `--echo-lines` since #188).
    ///
    /// Smaller than the echo ring on purpose: history keeps whole payloads so
    /// it can diff them, where echo keeps a one-line preview. Entries dropped
    /// to respect this bound are counted and displayed (RFC 09 §5.1 O6).
    #[arg(long)]
    pub history_entries: Option<usize>,

    /// How many distinct keys to keep statistics for (default
    /// `zenkey_fleet::DEFAULT_MAX_KEYS`; remembered since #188).
    ///
    /// Least-recently-seen keys are retired past this, and the retirements are
    /// counted and displayed — a long-running observer is bounded, and says so
    /// (RFC 09 §5.1 O6). Raise it on a bus with a very wide key population.
    #[arg(long)]
    pub max_keys: Option<usize>,
}

/// Resolved, owned settings. No `Option` gymnastics past this point.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Settings {
    /// The deployment base. Empty is legal and is the default (RFC v1.6).
    pub base: String,
    pub connect: Vec<String>,
    pub listen: Vec<String>,
    /// `None` = not stated by flag or context: the engine leaves a config
    /// file's choice alone and defaults off without one (#122).
    pub scouting: Option<bool>,
    /// The zenoh JSON5 passthrough file, when one is configured (#122).
    pub zenoh_config: Option<PathBuf>,
    pub registry: Vec<PathBuf>,
    pub timeout_secs: u64,
    pub scope: ScopePreset,
    pub selectors: Vec<String>,
    pub eager: bool,
    pub echo_lines: usize,
    pub history_entries: usize,
    pub max_keys: usize,
}

impl Cli {
    /// Resolve into settings, or explain why not: loads the active shared
    /// context (issue #35) and layers flags over it, with `prefs` supplying
    /// what the window remembered (issue #189).
    pub fn settings(self, prefs: &crate::prefs::Prefs) -> anyhow::Result<Settings> {
        let named = self.context.clone().or_else(|| prefs.context.clone());
        let context = match zenkey_explorer_config::active(named.as_deref()) {
            Ok(c) => c,
            // A name the user *typed* and that is missing is an error. A
            // remembered one that has since been deleted is a stale
            // preference, and refusing to start over it would be the worst
            // kind of persistence.
            Err(e) if self.context.is_some() => return Err(e),
            Err(_) => zenkey_explorer_config::active(None)?,
        };
        self.settings_with(context, prefs)
    }

    /// The pure half: `context` and `prefs` supply defaults, flags override.
    /// Split from [`Cli::settings`] so tests never touch the user's real
    /// config file.
    pub fn settings_with(
        self,
        context: Option<zenkey_explorer_config::StoredContext>,
        prefs: &crate::prefs::Prefs,
    ) -> anyhow::Result<Settings> {
        let context = context.unwrap_or_default();
        // A typed `--selector` is validated hard: the user just wrote it, and
        // a bad one is an error, not a preference to shrug off.
        for sel in &self.selector {
            crate::scope::validate_selector(sel)?;
        }
        // A remembered selector is filtered soft (the counterpart of
        // `Prefs::sanitised`, for a `Prefs` handed in directly): a stale or
        // hand-edited row must not refuse to start.
        let selectors: Vec<String> = if self.selector.is_empty() {
            prefs
                .selectors
                .iter()
                .filter(|s| crate::scope::validate_selector(s).is_ok())
                .cloned()
                .collect()
        } else {
            self.selector.clone()
        };
        // A `--selector` implies custom whatever else was asked for; then the
        // flag; then what the window had last time. Since #187 the selectors
        // persist beside the scope, so a remembered `custom` restores — it is
        // dropped only when nothing survived to give it meaning, because
        // refusing to start over a stale preference would be the worst kind
        // of persistence (issue #189).
        let scope = if !self.selector.is_empty() {
            ScopePreset::Custom
        } else {
            self.scope
                .or_else(|| {
                    Some(prefs.scope).filter(|s| *s != ScopePreset::Custom || !selectors.is_empty())
                })
                .unwrap_or(ScopePreset::Everything)
        };
        if scope == ScopePreset::Custom && selectors.is_empty() {
            anyhow::bail!("--scope custom needs at least one --selector");
        }
        // The bounds resolve flag > remembered (#188) > documented default.
        // The same soft/hard split as the selectors: a *typed* zero is an
        // error, a remembered zero was already dropped by `Prefs::sanitised`
        // and is re-dropped here for a `Prefs` handed in directly.
        let echo_lines = self
            .echo_lines
            .or(prefs.echo_lines.filter(|n| *n > 0))
            .unwrap_or(2000);
        let history_entries = self
            .history_entries
            .or(prefs.history_entries.filter(|n| *n > 0))
            .unwrap_or(200);
        let max_keys = self
            .max_keys
            .or(prefs.max_keys.filter(|n| *n > 0))
            .unwrap_or(zenkey_fleet::DEFAULT_MAX_KEYS);
        if echo_lines == 0 {
            anyhow::bail!("--echo-lines must be at least 1");
        }
        if history_entries == 0 {
            anyhow::bail!("--history-entries must be at least 1");
        }
        if max_keys == 0 {
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
            scouting: if self.scouting {
                Some(true)
            } else {
                context.scouting
            },
            zenoh_config: self.zenoh_config.or(context.zenoh_config),
            registry: if self.registry.is_empty() {
                context.registry
            } else {
                self.registry
            },
            timeout_secs: self.timeout.or(context.timeout).unwrap_or(5),
            scope,
            selectors,
            // The flag can only assert, so it wins by OR: `--eager` over a
            // remembered lazy is eager; there is no `--no-eager` to lose.
            eager: self.eager || prefs.eager.unwrap_or(false),
            echo_lines,
            history_entries,
            max_keys,
        })
    }
}

/// How a base should be shown to a human.
///
/// The empty base is a real, named deployment — the default one — not an
/// unset field. RFC 09 §5: "an explorer that cannot see and name it is
/// blind to the common case." One function, shared by [`Settings::base_label`]
/// and [`BaseChoice`]'s `Display`, so the window title and the picker rows
/// cannot drift into two spellings.
pub fn base_label(base: &str) -> &str {
    if base.is_empty() {
        "(empty — keys start at v1/)"
    } else {
        base
    }
}

/// One row of the base picker: a deployment base, displayed by its label.
///
/// The wrapper exists because a `pick_list` renders a `String` verbatim, which
/// is how the empty base — the RFC v1.6 bus-root deployment, a first-class
/// value — came to render as a literally blank row (#185).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseChoice {
    /// The base as the wire spells it; `""` is the bus root.
    pub base: String,
}

impl BaseChoice {
    pub fn new(base: impl Into<String>) -> BaseChoice {
        BaseChoice { base: base.into() }
    }

    /// The base-less bus-root deployment (RFC v1.6's default).
    pub fn bus_root() -> BaseChoice {
        BaseChoice {
            base: String::new(),
        }
    }
}

impl std::fmt::Display for BaseChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(base_label(&self.base))
    }
}

impl Settings {
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }

    /// How the base should be shown to a human — see [`base_label`].
    pub fn base_label(&self) -> &str {
        base_label(&self.base)
    }

    /// Whether this session can observe anything at all.
    ///
    /// With no endpoints and no scouting, zenoh opens a session that reaches
    /// nothing. The window would then show an empty tree, which is
    /// indistinguishable from a quiet bus — the false verdict of RFC 05 §3.1.
    /// The UI turns this into a banner rather than letting the user infer.
    pub fn is_unreachable(&self) -> bool {
        self.connect.is_empty()
            && self.listen.is_empty()
            && !self.scouting.unwrap_or(false)
            // A config file may carry endpoints or scouting of its own —
            // reachability is then the file's question, not the flags'.
            && self.zenoh_config.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::Prefs;

    fn parse(args: &[&str]) -> anyhow::Result<Settings> {
        // settings_with(None), not settings(): tests must never read the
        // user's real config file.
        Cli::try_parse_from(std::iter::once("zengui").chain(args.iter().copied()))?
            .settings_with(None, &crate::prefs::Prefs::default())
    }

    /// The same, over a remembered set of preferences.
    fn parse_remembering(args: &[&str], prefs: &crate::prefs::Prefs) -> anyhow::Result<Settings> {
        Cli::try_parse_from(std::iter::once("zengui").chain(args.iter().copied()))?
            .settings_with(None, prefs)
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

    /// …and the picker row a `pick_list` renders says the same thing, because
    /// it displays through the same label (#185). Before this type, the picker
    /// rendered the empty base as a literally blank row.
    #[test]
    fn a_base_picker_row_displays_the_label_not_the_raw_string() {
        assert_eq!(
            BaseChoice::bus_root().to_string(),
            "(empty — keys start at v1/)"
        );
        assert_eq!(BaseChoice::new("acme/fleet-a").to_string(), "acme/fleet-a");
        // The picker's `selected` comparison is by base, not by label.
        assert_eq!(BaseChoice::new(""), BaseChoice::bus_root());
    }

    #[test]
    fn defaults_match_the_documented_ones() {
        let s = parse(&[]).unwrap();
        assert_eq!(s.timeout(), Duration::from_secs(5));
        assert_eq!(s.scouting, None, "scouting is opt-in (RFC 09 §0.1)");
        assert_eq!(s.scope, ScopePreset::Everything);
        assert!(!s.eager, "lazy is the default (issue #85)");
        assert_eq!(s.echo_lines, 2000);
        assert_eq!(s.history_entries, 200);
        assert_eq!(s.max_keys, zenkey_fleet::DEFAULT_MAX_KEYS);
    }

    /// History retains whole payloads, so its bound is the one most worth
    /// turning down — and, like every other bound here, zero is rejected at
    /// the boundary rather than clamped downstream.
    #[test]
    fn the_history_bound_is_configurable_and_must_be_positive() {
        assert_eq!(
            parse(&["--history-entries", "5"]).unwrap().history_entries,
            5
        );
        assert!(parse(&["--history-entries", "0"]).is_err());
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

    /// The bounds the Settings overlay persists come back on the next
    /// launch, and a typed flag beats the memory without becoming it (#188).
    #[test]
    fn remembered_bounds_fill_in_and_a_flag_still_wins() {
        let remembered = Prefs {
            echo_lines: Some(5000),
            history_entries: Some(400),
            max_keys: Some(100_000),
            eager: Some(true),
            ..Prefs::default()
        };
        let s = parse_remembering(&[], &remembered).unwrap();
        assert_eq!(s.echo_lines, 5000);
        assert_eq!(s.history_entries, 400);
        assert_eq!(s.max_keys, 100_000);
        assert!(s.eager, "a remembered eager connects observing");

        let s = parse_remembering(&["--echo-lines", "100"], &remembered).unwrap();
        assert_eq!(s.echo_lines, 100, "the typed flag wins this launch");
        assert_eq!(s.max_keys, 100_000, "the others stay remembered");

        // A remembered zero is a hand edit: dropped to the default, never a
        // refusal — where the typed zero stays fatal (the tests above).
        let stale = Prefs {
            echo_lines: Some(0),
            ..Prefs::default()
        };
        let s = parse_remembering(&[], &stale).expect("must still start");
        assert_eq!(s.echo_lines, 2000);
    }

    /// Context supplies defaults; flags override (issue #35).
    #[test]
    fn context_supplies_defaults_and_flags_override() {
        let ctx = zenkey_explorer_config::StoredContext {
            base: Some("zensight".into()),
            connect: vec!["tcp/10.0.0.1:7447".into()],
            listen: vec![],
            registry: vec![],
            scouting: Some(true),
            timeout: Some(9),
            zenoh_config: None,
        };
        let cli = |args: &[&str]| {
            Cli::try_parse_from(std::iter::once("zengui").chain(args.iter().copied())).unwrap()
        };
        // No flags: the context wins everywhere.
        let s = cli(&[])
            .settings_with(Some(ctx.clone()), &Prefs::default())
            .unwrap();
        assert_eq!(s.base, "zensight");
        assert_eq!(s.connect, ["tcp/10.0.0.1:7447"]);
        assert_eq!(s.scouting, Some(true));
        assert_eq!(s.timeout_secs, 9);

        // #122: the passthrough file resolves flag > context, and its
        // presence alone makes the session reachable (the file may carry
        // endpoints of its own).
        let mut with_file = ctx.clone();
        with_file.zenoh_config = Some(PathBuf::from("/etc/zenoh/ctx.json5"));
        let mut s = cli(&[])
            .settings_with(Some(with_file), &Prefs::default())
            .unwrap();
        assert_eq!(
            s.zenoh_config.as_deref(),
            Some(std::path::Path::new("/etc/zenoh/ctx.json5"))
        );
        s.connect.clear();
        assert!(!s.is_unreachable(), "a config file may reach on its own");
        let s = cli(&["--zenoh-config", "/tmp/flag.json5"])
            .settings_with(Some(ctx.clone()), &Prefs::default())
            .unwrap();
        assert_eq!(
            s.zenoh_config.as_deref(),
            Some(std::path::Path::new("/tmp/flag.json5"))
        );
        // Flags override field-by-field, including an explicit empty base.
        let s = cli(&["--base", "", "-c", "tcp/127.0.0.1:7447", "--timeout", "2"])
            .settings_with(Some(ctx), &Prefs::default())
            .unwrap();
        assert_eq!(s.base, "");
        assert_eq!(s.connect, ["tcp/127.0.0.1:7447"]);
        assert_eq!(s.timeout_secs, 2);
    }

    /// What the window remembered fills in the defaults, and a flag always
    /// beats it — the precedence `prefs.rs`'s module doc has always promised
    /// and nothing implemented (issue #189).
    #[test]
    fn a_remembered_scope_is_a_default_and_a_flag_still_wins() {
        let remembered = Prefs {
            scope: ScopePreset::State,
            ..Prefs::default()
        };
        assert_eq!(
            parse_remembering(&[], &remembered).unwrap().scope,
            ScopePreset::State,
            "the scope the window was last on comes back"
        );
        assert_eq!(
            parse_remembering(&["--scope", "telemetry"], &remembered)
                .unwrap()
                .scope,
            ScopePreset::Telemetry,
            "a typed flag beats a remembered value"
        );
        assert_eq!(
            parse(&[]).unwrap().scope,
            ScopePreset::Everything,
            "with nothing remembered, the documented default"
        );
    }

    /// A remembered `custom` restores with its selectors (#187) — the
    /// acceptance that a custom selector set survives restart — and a typed
    /// flag still wins over every remembered value.
    #[test]
    fn a_remembered_custom_scope_restores_with_its_selectors() {
        let remembered = Prefs {
            scope: ScopePreset::Custom,
            selectors: vec!["demo/**".into()],
            ..Prefs::default()
        };
        let s = parse_remembering(&[], &remembered).unwrap();
        assert_eq!(s.scope, ScopePreset::Custom);
        assert_eq!(s.selectors, ["demo/**"]);

        // An explicit `--scope custom` rides the remembered selectors too —
        // it used to refuse to start without a `--selector` beside it.
        let s = parse_remembering(&["--scope", "custom"], &remembered).unwrap();
        assert_eq!(s.selectors, ["demo/**"]);

        // A typed `--selector` replaces the remembered set outright.
        let s = parse_remembering(&["--selector", "acme/**"], &remembered).unwrap();
        assert_eq!(s.scope, ScopePreset::Custom);
        assert_eq!(s.selectors, ["acme/**"]);

        // A preset flag wins the scope, and the selectors stay remembered
        // for the next switch back to custom.
        let s = parse_remembering(&["--scope", "state"], &remembered).unwrap();
        assert_eq!(s.scope, ScopePreset::State);
        assert_eq!(s.selectors, ["demo/**"]);

        // A remembered row that no longer validates is filtered soft — a
        // stale preference must never refuse a launch (a *typed* bad
        // selector stays fatal, `a_bad_selector_is_rejected_at_the_boundary`).
        let stale = Prefs {
            scope: ScopePreset::Custom,
            selectors: vec!["demo/$*/x".into(), "ok/**".into()],
            ..Prefs::default()
        };
        let s = parse_remembering(&[], &stale).unwrap();
        assert_eq!(s.scope, ScopePreset::Custom);
        assert_eq!(s.selectors, ["ok/**"]);
    }

    /// A remembered `custom` with nothing left to give it meaning is dropped
    /// rather than restored — refusing to start over a stale preference
    /// would be the worst kind of persistence (issue #189).
    #[test]
    fn a_remembered_custom_scope_does_not_strand_the_next_launch() {
        let stranded = Prefs {
            scope: ScopePreset::Custom,
            ..Prefs::default()
        };
        let s = parse_remembering(&[], &stranded).expect("must still start");
        assert_eq!(s.scope, ScopePreset::Everything);
        assert!(s.selectors.is_empty());

        // …while a selector on the command line still implies custom.
        let s = parse_remembering(&["--selector", "demo/**"], &stranded).unwrap();
        assert_eq!(s.scope, ScopePreset::Custom);
    }
}

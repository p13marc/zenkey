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
    /// The deployment namespace — the prefix before `zk2/` on every key of
    /// the deployment (core §1.6). `--base` is its v1 spelling, kept as an
    /// alias.
    ///
    /// The deployment's services run *in* it as their Zenoh session
    /// `namespace` and never spell it. zengui opens two sessions, as zenctl
    /// and zenwatch do: one **in** the namespace, which reads presence,
    /// descriptors and contracts the way the deployment's consumers do, and
    /// one in **none**, which watches wire selectors and so sees traffic from
    /// outside the deployment too (the tooling guide's O1) — which is what
    /// lets it spot a leak.
    ///
    /// Resolves `--namespace` > the namespace picked last (remembered) >
    /// `ZENGUI_BASE` > the context's `base` > empty. Empty is the bus-root
    /// deployment, whose keys start at `zk2/`. Namespaces actually in use are
    /// listed at runtime (the location bar's picker), so this flag is a
    /// shortcut, not a requirement.
    #[arg(long, visible_alias = "base", value_name = "NAMESPACE")]
    pub namespace: Option<String>,

    /// Use a named context from the shared explorer config
    /// (`~/.config/zenkey-explorer/config.toml`, with a read-fallback to the
    /// legacy zenctl location). Flags always override context values.
    /// Default: the file's `current` pointer, or the
    /// `ZENKEY_EXPLORER_CONTEXT`/`ZENCTL_CONTEXT` env vars.
    #[arg(long, value_name = "NAME")]
    pub context: Option<String>,

    /// Endpoint to connect to, repeatable (e.g. `tcp/127.0.0.1:7447`).
    ///
    /// The session is a zenoh client of these endpoints — no listener, no
    /// gossip (RFC 09 §5) — so one nothing answers is a link error, not an
    /// empty tree, and one that does not parse is refused by name.
    #[arg(long, short = 'c', value_name = "ENDPOINT")]
    pub connect: Vec<String>,

    /// Endpoint to listen on, repeatable. Listening makes the session a peer,
    /// which an explorer otherwise never is.
    #[arg(long, short = 'l', value_name = "ENDPOINT")]
    pub listen: Vec<String>,

    /// Enable multicast scouting.
    ///
    /// Off by default — with a --zenoh-config too, unless the file itself
    /// states `scouting.multicast.enabled` — and deliberately so: an explorer
    /// that multicast-scouts joins whatever mesh it can reach, which is how a
    /// throwaway session ends up in a live fleet. Note RFC 09 §0.1 — this is
    /// the *multicast* half only; gossip is a separate switch.
    #[arg(long)]
    pub scouting: bool,

    /// Contracts known offline, repeatable: an authoring file (`*.toml`), a
    /// directory of them, or a `.history` root — loaded as zenctl's
    /// `--contracts` loads them (spec §9.7, §8.5).
    ///
    /// They seed the contract store, so a revision held here is never
    /// retrieved from the bus; every other revision a descriptor names is
    /// retrieved from its holders (§8.4). Remembered when applied from the
    /// Settings overlay, in zengui's own preferences and never in the shared
    /// context; a flag wins for this launch without becoming the default.
    #[arg(long = "contracts", value_name = "PATH")]
    pub contracts: Vec<PathBuf>,

    /// v1's registry directories. Refused by name: zk2 reads contracts
    /// (`--contracts`), and a registry slice describes nothing a zk2 bus
    /// carries.
    #[arg(long, value_name = "DIR", hide = true)]
    pub registry: Vec<PathBuf>,

    /// Query timeout in seconds (default 5; a context may override the
    /// default, an explicit flag overrides the context).
    #[arg(long)]
    pub timeout: Option<u64>,

    /// Zenoh JSON5 config file (#122): the passthrough that reaches a
    /// secured bus (TLS, QUIC with certs, usrpwd, …). Loaded as the base
    /// layer; --connect/--listen/--scouting apply on top when given
    /// (flag > env > context > file). A file that sets a session namespace
    /// is refused — zengui sets the deployment's on the session that reads
    /// it, and none on the one that watches the wire. Only what the file
    /// states counts: no `mode` means the explorer's client session, no
    /// `scouting.multicast.enabled` means multicast off.
    #[arg(long, value_name = "FILE", env = "ZENGUI_ZENOH_CONFIG")]
    pub zenoh_config: Option<PathBuf>,

    /// What to watch. Defaults to the scope this window had last time, then
    /// to `everything` — a flag, when given, always wins (issue #189).
    /// `telemetry` is v1's name for `streams`, and still accepted.
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

    /// Ask the routers' admin space what every session declared — its
    /// `@/<zid>/<whatami>/{subscriber,publisher,queryable,querier,token}/**`
    /// records — at boot, on a reconnect or a namespace switch and with
    /// each admin sweep, so the tree can show a declared key no sample has
    /// reached yet.
    ///
    /// Off by default, because asking can freeze the router (#890): zenoh
    /// 1.10.1 sends those replies while it holds its routing tables' read
    /// lock, each reply takes that lock again, and a declaration or a new
    /// session from any other client in between deadlocks the router for
    /// good. Turn it on only against a bus nothing else is declaring on.
    /// A router under the deny block `zenctl acl gen` writes refuses them
    /// to every principal (core §11.1, 0.50, #894), and the admin pane then
    /// says nothing answered. Off, the tree is the contract skeleton and
    /// what was observed, and the admin pane says "not asked". Remembered
    /// when applied from the Settings overlay; a flag turns it on for this
    /// launch only.
    #[arg(long)]
    pub admin_declared: bool,

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
    /// The deployment namespace (core §1.6). Empty is legal and is the
    /// default: the bus-root deployment, whose keys start at `zk2/`.
    pub namespace: String,
    pub connect: Vec<String>,
    pub listen: Vec<String>,
    /// `None` = not stated by flag or context: the engine leaves a config
    /// file's choice alone and defaults off without one (#122).
    pub scouting: Option<bool>,
    /// The zenoh JSON5 passthrough file, when one is configured (#122).
    pub zenoh_config: Option<PathBuf>,
    /// Contracts known offline (`--contracts`, or the Settings overlay's
    /// remembered list): files, directories or `.history` roots.
    pub contracts: Vec<PathBuf>,
    pub timeout_secs: u64,
    pub scope: ScopePreset,
    pub selectors: Vec<String>,
    pub eager: bool,
    /// Whether the admin space's declaration records are asked
    /// (`--admin-declared`): off by default, since asking can freeze a
    /// zenoh 1.10.1 router (#890).
    pub admin_declared: bool,
    pub echo_lines: usize,
    pub history_entries: usize,
    pub max_keys: usize,
}

/// The words a `--registry` is refused with: the flag is v1's, and the zk2
/// counterpart is named, so the refusal is a pointer rather than a wall.
pub const REGISTRY_REFUSED: &str = "--registry is v1's registry slices, which describe nothing \
     a zk2 bus carries — zk2 reads contracts: --contracts <PATH> (an authoring file, a \
     directory of them, or a .history root)";

impl Cli {
    /// Resolve into settings, or explain why not: loads the active shared
    /// context (issue #35) and layers flags over it, with `prefs` supplying
    /// what the window remembered (issue #189) and the environment its one
    /// rung, `ZENGUI_BASE`.
    pub fn settings(self, prefs: &crate::prefs::Prefs) -> anyhow::Result<Settings> {
        // v1's flag, refused before the context is read: the pointer to
        // `--contracts` is never buried under another error.
        if !self.registry.is_empty() {
            anyhow::bail!("{REGISTRY_REFUSED}");
        }
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
        let env = std::env::var("ZENGUI_BASE").ok();
        self.settings_with(context, prefs, env.as_deref())
    }

    /// The pure half: `context`, `prefs` and `env_namespace` (`ZENGUI_BASE`)
    /// supply defaults, flags override. Split from [`Cli::settings`] so
    /// tests never touch the user's real config file or environment.
    pub fn settings_with(
        self,
        context: Option<zenkey_explorer_config::StoredContext>,
        prefs: &crate::prefs::Prefs,
        env_namespace: Option<&str>,
    ) -> anyhow::Result<Settings> {
        if !self.registry.is_empty() {
            anyhow::bail!("{REGISTRY_REFUSED}");
        }
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
        let namespace = namespace(
            self.namespace.as_deref(),
            prefs.namespace.as_deref(),
            env_namespace,
            context.base.as_deref(),
        )?;
        let contracts = contracts(self.contracts, &prefs.contracts)?;
        Ok(Settings {
            namespace,
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
            // A context's `registry` is parsed with the rest of the context
            // and read by nothing here: zk2 reads contracts, and contracts
            // are remembered by this window, never by the shared context.
            contracts,
            timeout_secs: self.timeout.or(context.timeout).unwrap_or(5),
            scope,
            selectors,
            // The flag can only assert, so it wins by OR: `--eager` over a
            // remembered lazy is eager; there is no `--no-eager` to lose.
            eager: self.eager || prefs.eager.unwrap_or(false),
            // Asserted the same way: a flag or the overlay's memory turns it
            // on, and nothing else does (#890).
            admin_declared: self.admin_declared || prefs.admin_declared.unwrap_or(false),
            echo_lines,
            history_entries,
            max_keys,
        })
    }
}

/// The deployment namespace: `--namespace` > the one picked last > the
/// environment > the context's `base` > empty (the bus root).
///
/// The same hard/soft split as the selectors. A namespace the session would
/// refuse is an error when it was typed, set in the environment or stored
/// in a context — someone named it for this launch — and dropped when it was
/// only remembered: a stale preference must never refuse a launch. An
/// explicitly empty namespace is a deployment, the bus root, and does not
/// fall through to the next rung.
pub fn namespace(
    flag: Option<&str>,
    remembered: Option<&str>,
    env: Option<&str>,
    stored: Option<&str>,
) -> anyhow::Result<String> {
    if let Some(ns) = flag {
        check_namespace(ns)?;
        return Ok(ns.to_string());
    }
    if let Some(ns) = remembered.filter(|ns| check_namespace(ns).is_ok()) {
        return Ok(ns.to_string());
    }
    let ns = env.or(stored).unwrap_or_default();
    check_namespace(ns)?;
    Ok(ns.to_string())
}

/// Whether the session would take `namespace` (`zenkey_fleet::check_namespace`):
/// a canonical, concrete key prefix. Empty is the bus root, and always fine.
pub fn check_namespace(namespace: &str) -> anyhow::Result<()> {
    if namespace.is_empty() {
        return Ok(());
    }
    zenkey_fleet::check_namespace(namespace).map_err(|e| anyhow::anyhow!("{e}"))
}

/// The contracts: `--contracts` > the list the Settings overlay applied
/// last > none. A typed path that is not there is refused, as zenctl
/// refuses it; a remembered one is dropped from this launch (stale, not
/// fatal) and stays in the preferences for when it comes back.
pub fn contracts(typed: Vec<PathBuf>, remembered: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    if typed.is_empty() {
        return Ok(remembered.iter().filter(|p| p.exists()).cloned().collect());
    }
    for p in &typed {
        if !p.exists() {
            anyhow::bail!("--contracts {}: no such file or directory", p.display());
        }
    }
    Ok(typed)
}

/// How a namespace should be shown to a human.
///
/// The empty namespace is a real, named deployment — the default one — not
/// an unset field: an explorer that cannot see and name it is blind to the
/// common case (RFC 09 §5). One function, shared by
/// [`Settings::namespace_label`] and [`NamespaceChoice`]'s `Display`, so the
/// window title and the picker rows cannot drift into two spellings.
pub fn namespace_label(namespace: &str) -> &str {
    if namespace.is_empty() {
        "(empty — keys start at zk2/)"
    } else {
        namespace
    }
}

/// One row of the namespace picker: a deployment namespace, displayed by its
/// label.
///
/// The wrapper exists because a `pick_list` renders a `String` verbatim, which
/// is how the empty one — the bus-root deployment, a first-class value —
/// came to render as a literally blank row (#185).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceChoice {
    /// The namespace as the wire spells it; `""` is the bus root.
    pub namespace: String,
}

impl NamespaceChoice {
    pub fn new(namespace: impl Into<String>) -> NamespaceChoice {
        NamespaceChoice {
            namespace: namespace.into(),
        }
    }

    /// The bus-root deployment: no namespace.
    pub fn bus_root() -> NamespaceChoice {
        NamespaceChoice {
            namespace: String::new(),
        }
    }
}

impl std::fmt::Display for NamespaceChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(namespace_label(&self.namespace))
    }
}

impl Settings {
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }

    /// How the namespace should be shown to a human — see
    /// [`namespace_label`].
    pub fn namespace_label(&self) -> &str {
        namespace_label(&self.namespace)
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
        // user's real config file, nor its environment.
        Cli::try_parse_from(std::iter::once("zengui").chain(args.iter().copied()))?.settings_with(
            None,
            &crate::prefs::Prefs::default(),
            None,
        )
    }

    /// The same, over a remembered set of preferences.
    fn parse_remembering(args: &[&str], prefs: &crate::prefs::Prefs) -> anyhow::Result<Settings> {
        Cli::try_parse_from(std::iter::once("zengui").chain(args.iter().copied()))?
            .settings_with(None, prefs, None)
    }

    #[test]
    fn unset_namespace_and_explicit_empty_namespace_agree() {
        assert_eq!(parse(&[]).unwrap().namespace, "");
        assert_eq!(parse(&["--namespace", ""]).unwrap().namespace, "");
        assert_eq!(
            parse(&["--namespace", "zensight"]).unwrap().namespace,
            "zensight"
        );
        // v1's spelling is an alias, not a second flag (#614, FL6a).
        assert_eq!(parse(&["--base", "spray"]).unwrap().namespace, "spray");
    }

    /// A namespace the session would refuse is refused at the boundary when
    /// it was typed — a wildcard is not a deployment.
    #[test]
    fn a_typed_namespace_the_session_would_refuse_is_an_error() {
        let e = parse(&["--namespace", "acme/*"]).unwrap_err().to_string();
        assert!(e.contains("acme/*"), "{e}");
    }

    /// The ladder (decided on #585): `--namespace` > the one picked last >
    /// `ZENGUI_BASE` > the context's `base` > empty — each rung reachable,
    /// an explicitly empty one standing, and a remembered one the session
    /// would refuse dropped rather than refusing the launch.
    #[test]
    fn the_namespace_climbs_flag_remembered_env_context_empty() {
        assert_eq!(
            namespace(Some("flag"), Some("mem"), Some("env"), Some("ctx")).unwrap(),
            "flag"
        );
        assert_eq!(
            namespace(None, Some("mem"), Some("env"), Some("ctx")).unwrap(),
            "mem"
        );
        assert_eq!(
            namespace(None, None, Some("env"), Some("ctx")).unwrap(),
            "env"
        );
        assert_eq!(namespace(None, None, None, Some("ctx")).unwrap(), "ctx");
        assert_eq!(namespace(None, None, None, None).unwrap(), "");
        assert_eq!(
            namespace(None, Some(""), Some("env"), Some("ctx")).unwrap(),
            "",
            "an explicitly empty namespace is the bus root, not a gap"
        );
        assert_eq!(
            namespace(None, Some("stale/*"), Some("env"), None).unwrap(),
            "env",
            "a remembered namespace the session would refuse is dropped"
        );
        assert!(namespace(None, None, Some("bad/$*"), None).is_err());

        // Through the whole resolution: the environment is an argument.
        let cli = |args: &[&str]| {
            Cli::try_parse_from(std::iter::once("zengui").chain(args.iter().copied())).unwrap()
        };
        let remembered = Prefs {
            namespace: Some("picked".into()),
            ..Prefs::default()
        };
        let ctx = zenkey_explorer_config::StoredContext {
            base: Some("ctx".into()),
            ..Default::default()
        };
        let s = cli(&[])
            .settings_with(Some(ctx.clone()), &remembered, Some("env"))
            .unwrap();
        assert_eq!(s.namespace, "picked");
        let s = cli(&[])
            .settings_with(Some(ctx.clone()), &Prefs::default(), Some("env"))
            .unwrap();
        assert_eq!(s.namespace, "env");
        let s = cli(&[])
            .settings_with(Some(ctx), &Prefs::default(), None)
            .unwrap();
        assert_eq!(s.namespace, "ctx");
    }

    /// v1's `--registry` is refused by name (#614, FL6a), and the refusal
    /// names the flag that replaced it.
    #[test]
    fn registry_is_refused_by_name_and_points_at_contracts() {
        let e = parse(&["--registry", "registry/"]).unwrap_err().to_string();
        assert!(e.contains("--registry"), "{e}");
        assert!(e.contains("--contracts"), "{e}");
    }

    /// `--contracts` is repeatable; a typed path that is not there is
    /// refused, as zenctl refuses it, while a remembered one that went
    /// stale is dropped from the launch — never a refusal.
    #[test]
    fn contracts_are_typed_hard_and_remembered_soft() {
        let here = env!("CARGO_MANIFEST_DIR");
        let s = parse(&["--contracts", here, "--contracts", here]).unwrap();
        assert_eq!(s.contracts.len(), 2);
        let e = parse(&["--contracts", "/definitely/not/here"])
            .unwrap_err()
            .to_string();
        assert!(e.contains("/definitely/not/here"), "{e}");

        let remembered = Prefs {
            contracts: vec![PathBuf::from(here), PathBuf::from("/definitely/not/here")],
            ..Prefs::default()
        };
        let s = parse_remembering(&[], &remembered).expect("a stale entry never refuses");
        assert_eq!(s.contracts, [PathBuf::from(here)]);
        // A flag wins this launch, and does not merge with the memory.
        let src = format!("{here}/src");
        let s = parse_remembering(&["--contracts", &src], &remembered).unwrap();
        assert_eq!(s.contracts, [PathBuf::from(&src)]);
    }

    /// The empty namespace is a deployment, not an unset field.
    #[test]
    fn the_empty_namespace_is_named_not_blank() {
        let s = parse(&[]).unwrap();
        assert!(s.namespace_label().contains("empty"));
        assert!(s.namespace_label().contains("zk2/"));
        assert_eq!(
            parse(&["--namespace", "zensight"])
                .unwrap()
                .namespace_label(),
            "zensight"
        );
    }

    /// …and the picker row a `pick_list` renders says the same thing, because
    /// it displays through the same label (#185). Before this type, the picker
    /// rendered the empty base as a literally blank row.
    #[test]
    fn a_namespace_picker_row_displays_the_label_not_the_raw_string() {
        assert_eq!(
            NamespaceChoice::bus_root().to_string(),
            "(empty — keys start at zk2/)"
        );
        assert_eq!(
            NamespaceChoice::new("acme/fleet-a").to_string(),
            "acme/fleet-a"
        );
        // The picker's `selected` comparison is by namespace, not by label.
        assert_eq!(NamespaceChoice::new(""), NamespaceChoice::bus_root());
    }

    #[test]
    fn defaults_match_the_documented_ones() {
        let s = parse(&[]).unwrap();
        assert_eq!(s.timeout(), Duration::from_secs(5));
        assert_eq!(s.scouting, None, "scouting is opt-in (RFC 09 §0.1)");
        assert_eq!(s.scope, ScopePreset::Everything);
        assert!(!s.eager, "lazy is the default (issue #85)");
        assert!(
            !s.admin_declared,
            "the declaration records are not asked by default (#890)"
        );
        assert_eq!(s.echo_lines, 2000);
        assert_eq!(s.history_entries, 200);
        assert_eq!(s.max_keys, zenkey_fleet::DEFAULT_MAX_KEYS);
        assert!(s.contracts.is_empty(), "contracts from the bus only");
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

    /// The declared-entities sweep is asked only when someone said so: the
    /// flag for this launch, or the Settings overlay's memory (#890).
    #[test]
    fn the_declared_sweep_is_asked_only_by_flag_or_memory() {
        assert!(parse(&["--admin-declared"]).unwrap().admin_declared);
        let on = Prefs {
            admin_declared: Some(true),
            ..Prefs::default()
        };
        assert!(parse_remembering(&[], &on).unwrap().admin_declared);
        let off = Prefs {
            admin_declared: Some(false),
            ..Prefs::default()
        };
        assert!(!parse_remembering(&[], &off).unwrap().admin_declared);
        assert!(
            parse_remembering(&["--admin-declared"], &off)
                .unwrap()
                .admin_declared,
            "the flag wins its launch"
        );
    }

    /// Context supplies defaults; flags override (issue #35).
    #[test]
    fn context_supplies_defaults_and_flags_override() {
        let ctx = zenkey_explorer_config::StoredContext {
            base: Some("zensight".into()),
            connect: vec!["tcp/10.0.0.1:7447".into()],
            listen: vec![],
            // Parsed with the context, and read by nothing (#614, FL6a).
            registry: vec!["/ctx/registry".into()],
            scouting: Some(true),
            timeout: Some(9),
            zenoh_config: None,
        };
        let cli = |args: &[&str]| {
            Cli::try_parse_from(std::iter::once("zengui").chain(args.iter().copied())).unwrap()
        };
        // No flags: the context wins everywhere.
        let s = cli(&[])
            .settings_with(Some(ctx.clone()), &Prefs::default(), None)
            .unwrap();
        assert_eq!(s.namespace, "zensight");
        assert_eq!(s.connect, ["tcp/10.0.0.1:7447"]);
        assert_eq!(s.scouting, Some(true));
        assert_eq!(s.timeout_secs, 9);
        assert!(
            s.contracts.is_empty(),
            "a context's registry is v1's slices, never read as contracts"
        );

        // #122: the passthrough file resolves flag > context, and its
        // presence alone makes the session reachable (the file may carry
        // endpoints of its own).
        let mut with_file = ctx.clone();
        with_file.zenoh_config = Some(PathBuf::from("/etc/zenoh/ctx.json5"));
        let mut s = cli(&[])
            .settings_with(Some(with_file), &Prefs::default(), None)
            .unwrap();
        assert_eq!(
            s.zenoh_config.as_deref(),
            Some(std::path::Path::new("/etc/zenoh/ctx.json5"))
        );
        s.connect.clear();
        assert!(!s.is_unreachable(), "a config file may reach on its own");
        let s = cli(&["--zenoh-config", "/tmp/flag.json5"])
            .settings_with(Some(ctx.clone()), &Prefs::default(), None)
            .unwrap();
        assert_eq!(
            s.zenoh_config.as_deref(),
            Some(std::path::Path::new("/tmp/flag.json5"))
        );
        // Flags override field-by-field, including an explicit empty base.
        let s = cli(&["--base", "", "-c", "tcp/127.0.0.1:7447", "--timeout", "2"])
            .settings_with(Some(ctx), &Prefs::default(), None)
            .unwrap();
        assert_eq!(s.namespace, "");
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
            ScopePreset::Streams,
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

//! `zenwatch` — the notifier beside the two explorers, on the same engine.
//!
//! zenctl explores a bus and zengui renders one; neither tells you when
//! something is wrong while you are asleep. This is the third binary: one
//! process, one config, N rules, M sinks. A rule is one spelling of the
//! closed [`rules::VOCABULARY`] — `zenctl watchdog`'s zk2 eight, its
//! `alert-firing` among them, plus `instance-gone <SYSTEM/SERVICE>` (each
//! address up or gone, read from the fleet's live presence projection by
//! the reading `zenctl watchdog`'s rule folds too, #747),
//! `liveliness-gone <SEL>` (the dead-man's switch on liveliness tokens) and
//! `alerts <SEL>` (each alarm of `alarms.v1` firing, resolved or
//! unobservable on its own, read by the fleet's live reader, #800 AD). A
//! transition on any rule produces exactly one notification per sink, and
//! the three states ride through — `unobservable` is never folded into
//! `ok`.
//!
//! **zk2 throughout** (#614): the read side — rules, presence, contracts,
//! the doctor — runs on the in-tree fleet, on two sessions, one in the
//! deployment's namespace and one in none ([`bus`], FL2). The write side
//! (FL3) makes the daemon a zk2 service on the namespaced session
//! ([`publish`]): `health.v1` through the runtime, and its own `zenwatch.v1`
//! contract (`contracts/`), whose typed server the build script generates
//! ([`zk2`]). Nothing here links v1's libraries.
//!
//! **A daemon, and not the rejected noun.** The redesign ledger
//! (`docs/redesign-2026-07.md` §6.1) rejected a hidden, auto-started,
//! shared-state background server whose job was caching discovery. This is
//! a daemon — it runs until stopped — and it is not that: it caches no
//! discovery, shares nothing with any explorer, and its only state is its
//! own notification ledger, bounded and reported.
//!
//! **Layering.** The presence projection is the engine's, because a
//! `.zrec` replay and a GUI pane want the same reading; which rule a move
//! is news to, and whether it is a duplicate, is this daemon's. The split
//! is visible in the module list: [`engine`] composes the fleet's watchdog
//! with the projection, its own token ledger and the fleet's alarm readers
//! and owns [`engine::route`]; [`presence`], [`liveliness`] and [`alarms`]
//! turn moves into per-rule notices;
//! [`doctor`] turns the engine's scheduled `run_doctor` into run-over-run
//! notices on that same stream (#390); [`sinks`] owns the daemon's wire
//! shapes ([`sinks::Outgoing`]); [`config`] owns the file and its
//! refusals; [`exit`] cites the contract.

pub mod alarms;
pub mod bus;
pub mod cli;
pub mod config;
pub mod discipline;
pub mod doctor;
pub mod engine;
pub mod exit;
pub mod liveliness;
pub mod presence;
pub mod publish;
pub mod render;
pub mod rules;
pub mod sinks;

/// The daemon's own contract, generated (#614, FL3): `zenwatch_v1`, from
/// `contracts/zenwatch.v1.toml` by zk2's `zenkey-build`, its JSON Schema
/// types bound to [`publish`]'s structs. Served by [`publish`].
pub mod zk2 {
    include!(concat!(env!("OUT_DIR"), "/zk2.rs"));
}

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser as _;
use zenkey_fleet::ContractSet;

use crate::bus::BusSettings;
use crate::cli::{Cli, Command, DeploymentArgs};
use crate::config::{Config, ConfigProblem};
use crate::exit::unaskable;

/// Refuse with every problem on stderr: exit 2, nothing run.
fn refuse(path: &Path, problems: &[ConfigProblem]) -> anyhow::Error {
    for p in problems {
        eprintln!("  {p}");
    }
    unaskable!(
        "{}: {} problem(s) — nothing was run",
        path.display(),
        problems.len()
    )
}

/// Load a config and refuse what the file alone refuses: a parse error or
/// any [`config::check`] problem is an exit 2, with every problem on
/// stderr. `test-sink` stops here; `run` and `check-config` go on to
/// [`load_checked`].
pub fn load_config(path: &Path) -> Result<Config> {
    let cfg = config::load(path).map_err(|e| unaskable!("{e}"))?;
    let problems = config::check(&cfg);
    if problems.is_empty() {
        Ok(cfg)
    } else {
        Err(refuse(path, &problems))
    }
}

/// A config with everything `run` resolves before it opens a session.
pub struct Loaded {
    pub cfg: Config,
    /// The contracts known offline: the config's `contracts`, then
    /// `--contracts`.
    pub contracts: ContractSet,
    /// The deployment catalogs: the config's `catalog` (core §8.6).
    pub catalogs: zenkey_fleet::Catalogs,
    /// The bus ladders, climbed: the namespace, the transport, the timeout.
    pub bus: BusSettings,
    /// The daemon's own service (#614, FL3), its address already resolved
    /// through the process's `hostid.v1` minter, before any session opens
    /// (§2.7); `None` under `publish: false`.
    pub publish: Option<zenkey::ServiceConfig>,
    /// The scope across a constrained face (`across_face`, #990 V7), and
    /// the engine rules' selectors narrowed to it: what the watchdog
    /// declares in place of the rules' own. `None` without the block.
    pub across: Option<Across>,
}

/// The `across_face` block, read (#990 V7; core §8.5, observer §7.5).
#[derive(Debug, Clone)]
pub struct Across {
    /// The fleet's scope: one selector per resource of the contracts in
    /// hand under each address, or what the face's plan lets cross.
    pub scope: zenkey_fleet::FaceScope,
    /// The engine rules' selectors narrowed to it, each declared once.
    pub selectors: Vec<String>,
}

/// [`load_config`], then what the deployment decides — the same refusals
/// for `run` and `check-config`, so the two cannot disagree (#614, FL2):
/// a context that names nothing, a namespace that is not one, a contracts
/// path that is not there or loads nothing, a state file that is malformed,
/// version 1 or another namespace's, and (FL3) a daemon whose own address
/// `hostid.v1` cannot mint on this host (§2.6) — minted here, once per
/// process, so `run` declares nothing before it is known. A file in a
/// contracts directory that is not a valid contract is said on stderr and
/// skipped, as zenctl does.
pub fn load_checked(path: &Path, dep: &DeploymentArgs) -> Result<Loaded> {
    let cfg = load_config(path)?;
    let mut problems = Vec::new();
    if let Some(ns) = &dep.namespace
        && let Err(e) = config::check_namespace(ns)
    {
        problems.push(ConfigProblem {
            path: "--namespace".into(),
            message: e,
        });
    }
    let sources: Vec<(String, PathBuf)> = cfg
        .contracts
        .iter()
        .enumerate()
        .map(|(i, p)| (format!("contracts[{i}]"), p.clone()))
        .chain(
            dep.contracts
                .iter()
                .map(|p| (format!("--contracts {}", p.display()), p.clone())),
        )
        .chain(cfg.across_face.iter().flat_map(|a| {
            a.contracts
                .iter()
                .enumerate()
                .map(|(i, p)| (format!("across_face.contracts[{i}]"), p.clone()))
        }))
        .collect();
    let contracts = load_contracts(&sources, &mut problems);
    // Core §8.6: each catalog read as a reader reads it; a refusal is the
    // config's problem, by where it was named.
    let mut held = Vec::new();
    for (i, path) in cfg.catalog.iter().enumerate() {
        match zenkey_fleet::Catalogs::load(path) {
            Ok(c) => held.push(c),
            Err(message) => problems.push(ConfigProblem {
                path: format!("catalog[{i}]"),
                message,
            }),
        }
    }
    let catalogs = zenkey_fleet::Catalogs::new(held);
    if !problems.is_empty() {
        return Err(refuse(path, &problems));
    }
    let mut bus = bus::resolve(&cfg.bus, dep.context.as_deref(), dep.namespace.as_deref())?;
    // `load_config` above refused `config::check`'s problems, the block's
    // static half among them; this is what needs the contracts loaded.
    let across = match &cfg.across_face {
        None => None,
        Some(a) => {
            let across = across_face(a, &cfg, &contracts, &bus.namespace)
                .map_err(|problems| refuse(path, &problems))?;
            // Core §8.1: a reply across the face takes at least its
            // `reply_timeout_ms`.
            if let Some(far) = across.scope.far() {
                bus.timeout = bus
                    .timeout
                    .max(std::time::Duration::from_millis(far.reply_timeout_ms));
                bus.far = Some(far.clone());
            }
            Some(across)
        }
    };
    if let Some(state) = &cfg.state_file
        && let Err(e) = discipline::state::load(state, &bus.namespace)
    {
        problems.push(ConfigProblem {
            path: "state_file".into(),
            message: e.to_string(),
        });
        return Err(refuse(path, &problems));
    }
    let publish = match cfg.publish.settings() {
        None => None,
        Some(s) => {
            let config = publish::service_config(s).map_err(|e| unaskable!("publish: {e}"))?;
            config
                .resolve()
                .map_err(|e| unaskable!("{}", publish::unminted(&e)))?;
            Some(config)
        }
    };
    Ok(Loaded {
        cfg,
        contracts,
        catalogs,
        bus,
        publish,
        across,
    })
}

/// The `across_face` block's scope and the engine rules' selectors narrowed
/// to it (#990 V7): refused, as a problem by where it was named, a policy
/// that cannot be read or that the linter refuses, a face the far side
/// does not read, a scope the fleet refuses (no contract, the plan's
/// errors, nothing in it), an address that is no `SYSTEM/SERVICE`, and an
/// engine rule whose selector meets none of the scope (observer §7.5:
/// never dropped silently).
fn across_face(
    a: &config::AcrossFaceConfig,
    cfg: &config::Config,
    contracts: &ContractSet,
    namespace: &str,
) -> std::result::Result<Across, Vec<ConfigProblem>> {
    let mut problems = Vec::new();
    let mut problem = |path: &str, message: String| {
        problems.push(ConfigProblem {
            path: path.to_owned(),
            message,
        });
    };
    let mut addresses: Vec<zenkey_model::grammar::Addr> = Vec::new();
    for (i, s) in a.addresses.iter().enumerate() {
        match s.parse() {
            Ok(addr) => addresses.push(addr),
            Err(e) => problem(
                &format!("across_face.addresses[{i}]"),
                format!("{s:?}: {e}. Name one service, SYSTEM/SERVICE, each a plain chunk"),
            ),
        }
    }
    let policy = match &a.policy {
        None => None,
        Some(p) => {
            let text = match std::fs::read_to_string(p) {
                Ok(t) => t,
                Err(e) => {
                    problem("across_face.policy", format!("{}: {e}", p.display()));
                    return Err(problems);
                }
            };
            let parsed = zenkey_model::link::parse(&text);
            match parsed.policy {
                Some(policy) => Some(policy),
                None => {
                    let errors: Vec<String> = parsed
                        .findings
                        .iter()
                        .filter(|f| f.is_error())
                        .map(ToString::to_string)
                        .collect();
                    problem(
                        "across_face.policy",
                        format!(
                            "{}: the linter refuses it ({}); `zenctl check face` names every \
                             finding",
                            p.display(),
                            errors.join("; ")
                        ),
                    );
                    return Err(problems);
                }
            }
        }
    };
    let face = policy
        .as_ref()
        .zip(a.face.as_deref())
        .map(|(policy, name)| zenkey_fleet::model::face_scope::Face { policy, name });
    let scope = match zenkey_fleet::FaceScope::of(
        &addresses,
        contracts,
        namespace,
        zenkey_fleet::model::face_scope::Reads::Samples,
        face,
    ) {
        Ok(s) => s,
        Err(e) => {
            problem("across_face", e.to_string());
            return Err(problems);
        }
    };
    // Each engine rule's selector, a wire key as the scope's are, narrowed
    // to it; one that meets none is refused by its rule.
    let mut selectors: Vec<String> = Vec::new();
    for (i, r) in cfg.rules.iter().enumerate() {
        let Ok(rules::RuleKind::Engine(c)) = rules::parse_rule(&r.rule) else {
            continue;
        };
        let Some(sel) = c.selector() else { continue };
        match scope.narrow(&[sel.to_owned()]) {
            Err(e) => problem(
                &format!("rules[{i}].rule"),
                format!("{}: not a key expression ({})", e.selector, e.error),
            ),
            Ok(n) if !n.unmet.is_empty() || n.selectors.is_empty() => problem(
                &format!("rules[{i}].rule"),
                format!(
                    "{sel} reaches none of the selectors across the face ({}): type a \
                     selector under one of them",
                    scope.selectors().join(" + ")
                ),
            ),
            Ok(n) => {
                for s in n.selectors {
                    if !selectors.contains(&s) {
                        selectors.push(s);
                    }
                }
            }
        }
    }
    if problems.is_empty() {
        Ok(Across { scope, selectors })
    } else {
        Err(problems)
    }
}

/// The contracts `sources` name, loaded once (spec §9.7, §8.5): a path
/// that is not there, or that loads nothing, is a problem by where it was
/// named; a file in a named directory that is not a valid contract is
/// said on stderr and skipped.
fn load_contracts(sources: &[(String, PathBuf)], problems: &mut Vec<ConfigProblem>) -> ContractSet {
    let mut set = ContractSet::new();
    for (at, path) in sources {
        if !path.exists() {
            problems.push(ConfigProblem {
                path: at.clone(),
                message: format!("{}: no such file or directory", path.display()),
            });
            continue;
        }
        let (loaded, skipped) = ContractSet::load_path(path);
        for p in &skipped {
            let mut lines = p.message.lines();
            eprintln!("contracts: {}: {}", p.at, lines.next().unwrap_or_default());
            for l in lines {
                eprintln!("  {l}");
            }
        }
        if loaded.is_empty() {
            problems.push(ConfigProblem {
                path: at.clone(),
                message: format!(
                    "{}: no contract loaded from it{}",
                    path.display(),
                    if skipped.is_empty() {
                        " (no authoring file, and not a .history root with bundles)"
                    } else {
                        " (see above)"
                    }
                ),
            });
            continue;
        }
        set.extend(loaded);
    }
    set
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::CheckConfig(a) => {
            let loaded = load_checked(&a.config.config, &a.deployment)?;
            let cfg = &loaded.cfg;
            println!(
                "{}: {} rule(s), {} sink(s){}{}{} — ok",
                a.config.config.display(),
                cfg.rules.len(),
                cfg.sinks.len(),
                match loaded.contracts.len() {
                    0 => String::new(),
                    n => format!(", {n} contract revision(s)"),
                },
                match &cfg.doctor {
                    Some(d) => format!(", doctor every {}", d.every_spelled()),
                    None => String::new(),
                },
                match &loaded.publish {
                    Some(p) => format!(", published as {}", p.address),
                    None => ", no service (publish: false)".to_string(),
                }
            );
            Ok(())
        }
        Command::TestSink(a) => {
            let cfg = load_config(&a.config.config)?;
            let Some(sc) = cfg.sinks.get(&a.name) else {
                return Err(unaskable!(
                    "sink {:?} is not in {} (which has: {})",
                    a.name,
                    a.config.config.display(),
                    cfg.sinks.keys().cloned().collect::<Vec<_>>().join(", ")
                ));
            };
            let sink = sinks::Sink::build(&a.name, sc, &sinks::http::client())
                .map_err(|e| unaskable!("{e}"))?;
            match sink.probe().await {
                Ok(note) => eprintln!("test-sink {}: probe — {note}", a.name),
                Err(e) => eprintln!("test-sink {}: probe — {e}", a.name),
            }
            let outgoing = sinks::test_outgoing(&a.name);
            let d = sink
                .deliver(&outgoing)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))
                .with_context_sink(&a.name, sink.kind_name())?;
            println!(
                "test-sink {}: delivered via {} — {}",
                a.name,
                sink.kind_name(),
                d.detail
            );
            Ok(())
        }
        Command::Run(a) => engine::run(a).await,
    }
}

/// A small context helper so a failed test delivery names the sink and
/// its kind in the `Error:` line, with the sink's own message as the cause.
trait WithSink<T> {
    fn with_context_sink(self, name: &str, kind: &str) -> Result<T>;
}

impl<T> WithSink<T> for Result<T> {
    fn with_context_sink(self, name: &str, kind: &str) -> Result<T> {
        use anyhow::Context as _;
        self.with_context(|| format!("test-sink {name}: delivery via {kind} failed"))
    }
}

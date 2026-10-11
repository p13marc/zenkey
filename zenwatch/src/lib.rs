//! `zenwatch` — the notifier beside the two explorers, on the same engine.
//!
//! zenctl explores a bus and zengui renders one; neither tells you when
//! something is wrong while you are asleep. This is the third binary: one
//! process, one config, N rules, M sinks. A rule is one spelling of the
//! closed [`rules::VOCABULARY`] — `zenctl watchdog`'s zk2 seven, plus
//! `instance-gone <SYSTEM/SERVICE>` (each address up or gone, read from the
//! fleet's live presence projection by the reading `zenctl watchdog`'s rule
//! folds too, #747) and `liveliness-gone <SEL>` (the
//! dead-man's switch on liveliness tokens). `alerts` is dark until zk2's
//! alarm profile exists (#613). A transition on any rule produces exactly
//! one notification per sink, and the three states ride through —
//! `unobservable` is never folded into `ok`.
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
//! with the projection and its own token ledger and owns [`engine::route`];
//! [`presence`] and [`liveliness`] turn moves into per-rule notices;
//! [`doctor`] turns the engine's scheduled `run_doctor` into run-over-run
//! notices on that same stream (#390); [`sinks`] owns the daemon's wire
//! shapes ([`sinks::Outgoing`]); [`config`] owns the file and its
//! refusals; [`exit`] cites the contract.

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
    /// The bus ladders, climbed: the namespace, the transport, the timeout.
    pub bus: BusSettings,
    /// The daemon's own service (#614, FL3), its address already resolved
    /// through the process's `hostid.v1` minter, before any session opens
    /// (§2.7); `None` under `publish: false`.
    pub publish: Option<zenkey::ServiceConfig>,
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
        .collect();
    let contracts = load_contracts(&sources, &mut problems);
    if !problems.is_empty() {
        return Err(refuse(path, &problems));
    }
    let bus = bus::resolve(&cfg.bus, dep.context.as_deref(), dep.namespace.as_deref())?;
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
        bus,
        publish,
    })
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

//! `zenwatch` — the notifier beside the two explorers, on the same engine.
//!
//! zenctl explores a bus and zengui renders one; neither tells you when
//! something is wrong while you are asleep. This is the third binary: one
//! process, one config, N rules, M sinks. A rule is one spelling of the
//! closed [`rules::VOCABULARY`] — `zenctl watchdog`'s eight, plus
//! `alerts <SEL>` (the producers' own alert documents, RFC 04 §1.2) and
//! `liveliness-gone <SEL>` (the dead-man's switch, RFC 04 §5). A
//! transition on any rule produces exactly one notification per sink, and
//! the three states ride through — `unobservable` is never folded into
//! `ok`.
//!
//! **A daemon, and not the rejected noun.** The redesign ledger
//! (`docs/redesign-2026-07.md` §6.1) rejected a hidden, auto-started,
//! shared-state background server whose job was caching discovery. This is
//! a daemon — it runs until stopped — and it is not that: it caches no
//! discovery, shares nothing with any explorer, and its only state is its
//! own notification ledger, bounded and reported.
//!
//! **Layering.** The alert projection (`AlertTransition`, RFC 11 §3.2's
//! alert ref) is the engine's, because a `.zrec` replay and a GUI pane want
//! the same reading; routing — which rule, which sink, whether it is a
//! duplicate — is this daemon's. The split is visible in the module list:
//! [`engine`] composes the fleet's watchdog with its own monitor and owns
//! [`engine::route`]; [`sinks`] owns the daemon's wire shapes
//! ([`sinks::Outgoing`]); [`config`] owns the file and its refusals;
//! [`exit`] cites the contract.

pub mod bus;
pub mod cli;
pub mod config;
pub mod discipline;
pub mod engine;
pub mod exit;
pub mod publish;
pub mod render;
pub mod rules;
pub mod sinks;

/// The daemon's own registry (RFC 08 §5), compiled through `zenkey-build`
/// like every producer's: `health`, `firing/{rule_id}`, `doctor`, and the
/// two procedures. Served and published by [`publish`] (#389).
pub mod registry {
    include!(concat!(env!("OUT_DIR"), "/zenkey_registry.rs"));
}

use std::path::Path;

use anyhow::Result;
use clap::Parser as _;

use crate::cli::{Cli, Command};
use crate::exit::unaskable;

/// Load a config and refuse it the way `run` would: a parse error or any
/// [`config::check`] problem is an exit 2, with every problem on stderr.
pub fn load_checked(path: &Path) -> Result<config::Config> {
    let cfg = config::load(path).map_err(|e| unaskable!("{e}"))?;
    let problems = config::check(&cfg);
    if problems.is_empty() {
        return Ok(cfg);
    }
    for p in &problems {
        eprintln!("  {p}");
    }
    Err(unaskable!(
        "{}: {} problem(s) — nothing was run",
        path.display(),
        problems.len()
    ))
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::CheckConfig(a) => {
            let cfg = load_checked(&a.config)?;
            println!(
                "{}: {} rule(s), {} sink(s) — ok",
                a.config.display(),
                cfg.rules.len(),
                cfg.sinks.len()
            );
            Ok(())
        }
        Command::TestSink(a) => {
            let cfg = load_checked(&a.config.config)?;
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

//! The clap tree: three verbs, one config file.
//!
//! `run` is the daemon; `check-config` is `run`'s refusal path on its own,
//! for CI and for editing a config with the daemon still up; `test-sink`
//! sends one synthetic notification through one configured sink, because a
//! sink that was never exercised is a sink that pages nobody at 3am. Every
//! verb takes the same `--config` (or `ZENWATCH_CONFIG`), and nothing else
//! about the bus or the rules is a flag: the config is the whole statement of
//! what this process does, and a flag that overrode part of it would make
//! the file a lie.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "zenwatch",
    version,
    about = "Notify when something is wrong on a keyspace-v2 Zenoh bus (#388)",
    long_about = "Watch N rules and route every genuine state change to M sinks.\n\n\
        Rules are the closed `zenctl watchdog` vocabulary — rate-above, rate-below, \
        silent-for, invalid-payload, qos-mismatch, doctor, origin-down, dropped — plus \
        `alerts <SEL>` (the producers' own alert documents: a put is firing, a delete is \
        resolved) and `liveliness-gone <SEL>` (the dead-man's switch on alive tokens). \
        Three states ride every notification, never two: ok / firing / unobservable, \
        and a drop in the observer is unobservable, never ok (RFC 13 §3 O6).\n\n\
        Exit codes: 0 clean, 1 the act failed, 2 refused input (`zenwatch/src/exit.rs`)."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the notifier: one process, one config, N rules, M sinks — until SIGINT/SIGTERM
    Run(RunArgs),
    /// Parse and validate a config, refusing exactly what `run` would refuse — exit 0 or 2
    CheckConfig(ConfigArgs),
    /// Send one synthetic notification through one configured sink — exit 0 delivered, 1 failed, 2 unknown sink
    TestSink(TestSinkArgs),
}

#[derive(Args, Debug, Clone)]
pub struct ConfigArgs {
    /// The JSON5 config file
    #[arg(long, env = "ZENWATCH_CONFIG", value_name = "FILE")]
    pub config: PathBuf,
}

#[derive(Args, Debug, Clone)]
pub struct RunArgs {
    #[command(flatten)]
    pub config: ConfigArgs,
    /// Print every notification that would be sent, as one ndjson line per sink, and send nothing
    #[arg(long)]
    pub dry_run: bool,
    /// The named connection context to run against (overrides the config's `bus.context`)
    #[arg(long, value_name = "NAME")]
    pub context: Option<String>,
    /// Evaluate one tick and exit — a smoke run, not a daemon
    #[arg(long)]
    pub once: bool,
}

#[derive(Args, Debug, Clone)]
pub struct TestSinkArgs {
    #[command(flatten)]
    pub config: ConfigArgs,
    /// The sink's name in the config's `sinks` table
    #[arg(value_name = "NAME")]
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The clap tree stays well-formed — the assertion clap itself offers.
    #[test]
    fn the_tree_is_well_formed() {
        use clap::CommandFactory as _;
        Cli::command().debug_assert();
    }
}

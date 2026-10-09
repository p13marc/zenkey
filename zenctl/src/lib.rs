//! `zenctl` — a bus explorer for zk2 deployments (#612).
//!
//! zenctl is app-neutral: nothing application-specific is compiled in, and
//! any zk2 deployment is explorable. What it knows of one it reads off the
//! bus — presence (instance and interface tokens, spec §8.1), the
//! descriptor each instance serves (§3.3), and the contract bundles those
//! descriptors name, retrieved by fingerprint (§8.4) — or from contracts
//! known offline (`--contracts`).
//!
//! Two kinds of verb, by session (decided 2026-10-08):
//!
//! * a **resolved** verb — `service`, `iface`, `schema`, `graph`, `compat`,
//!   `call`, `get state`, `watch`, `check expect|probe|schema|conform`,
//!   `why`, `doctor` and `admin graph`'s instance join —
//!   reads through a session opened **in** the deployment's namespace
//!   (`--namespace`, alias `--base`; the active context's `base`), as the
//!   deployment's own consumers do;
//! * a **raw** verb — `get <SELECTOR>`, `echo`, `rate`, `field`, `record`,
//!   `timeline`, `snapshot`, `pub`, `replay`, `admin`, `storage`,
//!   `namespace list`, `scout` — runs on a session in **no** namespace and
//!   sees the wire as it is. The observers among them resolve each key
//!   through one lens (FJ8b): the namespace, its presence read, the
//!   contract each descriptor names.
//!
//! v1 left at FJ9: the registry (`--registry`, RFC 08 §6 introspection, the
//! slice cache), the v1 judges and the profile-backed nouns. The `v1`
//! branch keeps them, and releases 0.14.x.
//!
//! Two seams carry the shape of the tool rather than the shape of a command:
//! [`cli`] is the clap tree and the vocabulary it enforces (#307), and
//! [`exit`] is the exit contract every verb cites. Read those two and the
//! rest is verbs.
pub(crate) mod bus;
pub mod cli;
pub mod errors;
pub mod exit;
pub mod input;
pub mod render;
pub mod report;
pub(crate) mod resolve;

mod cmd;
mod completion;
mod context;

use anyhow::Result;

// `cmd/*` runs against the **resolved** flags, never the parsed ones: the
// context is read once, at the edge, and a command that gets a `Link` or a
// `Deployment` is past every way resolution can fail (#209).
use crate::cli::{
    AclCmd, AdminCmd, BenchCmd, CheckCmd, Cli, Command, GetSub, IfaceCmd, KeyCmd, NamespaceCmd,
    SchemaCmd, ServiceCmd, SnapshotSub, StorageCmd,
};

/// Parse, through `get_matches` rather than `parse()`.
///
/// The extra step buys one thing: the `ArgMatches` remember *how* each value
/// arrived, which is what lets `--format` conflict with a foreign document
/// format only when both were typed — an exported `ZENCTL_FORMAT` is a
/// preference, not a request (#243, and `cli::refuse_foreign_format`).
fn parse() -> Cli {
    let matches = <Cli as clap::CommandFactory>::command().get_matches();
    cli::refuse_foreign_format(&matches);
    cli::refuse_stream_json(&matches);
    match <Cli as clap::FromArgMatches>::from_arg_matches(&matches) {
        Ok(cli) => cli,
        // Unreachable in practice: `get_matches` has already exited on a bad
        // command line, so anything left is a derive bug, and printing it the
        // way clap prints its own errors is the most useful thing to do.
        Err(e) => e.exit(),
    }
}

pub async fn run() -> Result<()> {
    // Behave like a Unix filter under `zenctl … | head`: Rust masks SIGPIPE,
    // turning a closed pipe into a mid-write panic; restore the default
    // disposition so the process exits quietly (141) instead.
    #[cfg(unix)]
    // SAFETY: installing SIG_DFL (not a handler fn) is process-wide and
    // has no safety obligations beyond the FFI call itself.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    // Before anything else, and before parsing: a completion request arrives
    // as a *partial* command line the parser would reject (issue #54). This
    // exits the process when it is one, and is a no-op otherwise.
    completion::maybe_serve();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = parse();
    match cli.command {
        // ── Nouns ────────────────────────────────────────────────────────
        Command::Service(ServiceCmd::List(a)) => cmd::service::list(a).await,
        Command::Service(ServiceCmd::Show(a)) => cmd::service::show(a).await,
        Command::Iface(IfaceCmd::List(a)) => cmd::iface::list(a).await,
        Command::Iface(IfaceCmd::Show(a)) => cmd::iface::show(a).await,
        Command::Namespace(NamespaceCmd::List(a)) => cmd::namespace::list(a).await,
        Command::Schema(SchemaCmd::Show(a)) => cmd::schema::show(a).await,
        Command::Storage(StorageCmd::List(a)) => cmd::storage::list(a).await,
        Command::Storage(StorageCmd::Gen(a)) => cmd::storage::plan(a).await,
        Command::Acl(AclCmd::Gen(a)) => cmd::acl::run(a).await,
        Command::Admin(AdminCmd::Routers { session }) => {
            let link = crate::bus::Link::resolve(&session)?;
            cmd::admin::routers(&link).await
        }
        Command::Admin(AdminCmd::Graph(a)) => cmd::admin::graph(a).await,
        Command::Key(KeyCmd::Includes(a)) => cmd::key::includes(a),
        Command::Key(KeyCmd::Intersects(a)) => cmd::key::intersects(a),
        Command::Key(KeyCmd::Canon { expr, out }) => cmd::key::canon(&expr, out.format, out.color),
        Command::Bench(BenchCmd::Call(a)) => cmd::bench::call(a).await,

        // ── Wire verbs ───────────────────────────────────────────────────
        Command::Get(a) => match a.cmd {
            Some(GetSub::State(s)) => cmd::get::state(*s).await,
            None => cmd::get::run(a).await,
        },
        Command::Call(a) => cmd::call::run(a).await,
        Command::Watch(a) => cmd::subscribe::run(a).await,
        Command::Echo(a) => cmd::echo::run(a).await,
        Command::Pub(a) => cmd::publish::dispatch(a).await,
        Command::Rate(a) => cmd::rate::run(a).await,
        Command::Field(a) => cmd::field::run(a).await,
        Command::Record(a) => cmd::record::run(a).await,
        Command::Replay(a) => cmd::replay::run(a).await,
        Command::Timeline(a) => cmd::timeline::run(a).await,
        Command::Snapshot(a) => match a.cmd {
            Some(SnapshotSub::Diff(d)) => cmd::snapshot::diff(d),
            None => cmd::snapshot::take(a).await,
        },
        Command::Graph(a) => cmd::graph::run(a).await,
        Command::Compat(a) => cmd::compat::run(a).await,
        Command::Serve(a) => cmd::serve::run(a).await,
        Command::Gen(a) => cmd::generate::run(a).await,
        Command::Scout(a) => cmd::scout::run(a).await,

        // ── Judgement ────────────────────────────────────────────────────
        Command::Check(CheckCmd::Expect(a)) => cmd::expect::run(a).await,
        Command::Check(CheckCmd::Probe(a)) => cmd::probe::run(a).await,
        Command::Check(CheckCmd::Schema(a)) => cmd::schema::check(a).await,
        Command::Check(CheckCmd::Conform(a)) => cmd::conform::run(a).await,
        Command::Why(a) => cmd::why::run(a).await,
        Command::Doctor(a) => cmd::doctor::run(a).await,
        Command::Watchdog(a) => cmd::watchdog::run(a).await,

        // ── Meta ─────────────────────────────────────────────────────────
        Command::Context(cmd) => context::dispatch(cmd),
        Command::Cache(cmd) => cmd::cache::dispatch(cmd).await,
        Command::Completions { shell, static_only } => completion::emit(shell, static_only),
    }
}

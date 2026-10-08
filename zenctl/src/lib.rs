//! `zenctl` — a bus explorer for the keyspace-v2 convention.
//!
//! RFC 08 §6 names this tool: runtime introspection exists so that "generic
//! explorer tooling — the `busctl`/`d-feet` equivalent — needs no compiled-in
//! registry". Every producer MUST serve `@rpc/<producer>/introspect`, so a
//! fleet can describe itself. `zenctl` is app-neutral: nothing
//! application-specific is compiled in, and any conformant fleet is
//! explorable.
//!
//! Registry knowledge comes from one of two sources:
//!
//! * **the live bus** (the default): each producer's served `introspect`
//!   slice — what the fleet *actually* serves;
//! * **`--registry <dir>`** (repeatable): local `registry/*.{toml,kdl}` files — what
//!   a checked-out application *declares*. Works with the fleet down.
//!
//! The gap between the two is drift, and `doctor` (bus + `--registry`) is the
//! command that reports it.
//!
//! **zk2's nouns** (#612, FJ4) — `service`, `iface`, `schema`, `namespace`,
//! and the `graph` and `compat` verbs — read no slice at all: presence
//! tokens, the descriptor each instance serves, and contract bundles,
//! through a session opened in the deployment's namespace (`--namespace`,
//! alias `--base`; decided 2026-10-08), with `--contracts` for revisions
//! known offline. They replaced v1's `topic`, `node`, `base`, `interface`
//! and `registry`; the rest of the tree is ported verb by verb until FJ9.
//! FJ5 added the acts and reads through a contract — `call` (which
//! replaced v1's `service call`), `get state` and `watch` — and the P3
//! guard on `pub`; `retire` is gone, and `replay --namespace` writes
//! through a session in a deployment namespace.
//!
//! Two seams carry the shape of the tool rather than the shape of a command:
//! [`cli`] is the clap tree and the vocabulary it enforces (#307), and
//! [`exit`] is the exit contract every verb cites. Read those two and the
//! rest is verbs.
pub(crate) mod bus;
pub mod cli;
pub(crate) mod degrade;
pub mod errors;
pub mod exit;
pub mod input;
pub mod render;
pub mod report;
pub(crate) mod resolve;

mod cmd;
/// The one HTTP route `export` serves (#228), reachable so its test can bind
/// it around a fixed body without a bus.
pub use cmd::export::http as export_http;
mod completion;
mod context;

use anyhow::Result;

/// `cmd/*` runs against the **resolved** flags, never the parsed ones: the
/// context is read once, at the edge, and a command that gets a [`Bus`] is
/// past every way resolution can fail (#209).
pub(crate) use crate::bus::Bus;
use crate::cli::{
    AclCmd, AdminCmd, BenchCmd, BlobCmd, CheckCmd, Cli, Command, ConfigCmd, GetSub, IfaceCmd,
    KeyCmd, NamespaceCmd, SchemaCmd, ServiceCmd, SnapshotSub, StorageCmd,
};

/// Parse, through `get_matches` rather than `parse()`.
///
/// The extra step buys one thing: the `ArgMatches` remember *how* each value
/// arrived, which is what lets `--format` conflict with a foreign document
/// format only when both were typed — an exported `ZENCTL_FORMAT` is a
/// preference, not a request (#243, and `cli::refuse_foreign_format`).
/// The second value is [`cli::gen_target_typed`]'s answer, carried out of the
/// one scope that still holds the `ArgMatches`: whether the bus target was
/// typed on this command line, which is the half of `gen --fault`'s double
/// guard the derive struct cannot answer (#163 — clap folds `ZENCTL_BASE` in
/// before the struct exists).
fn parse() -> (Cli, bool) {
    let matches = <Cli as clap::CommandFactory>::command().get_matches();
    cli::refuse_foreign_format(&matches);
    cli::refuse_stream_json(&matches);
    let target_typed = cli::gen_target_typed(&matches);
    match <Cli as clap::FromArgMatches>::from_arg_matches(&matches) {
        Ok(cli) => (cli, target_typed),
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

    let (cli, gen_target_typed) = parse();
    match cli.command {
        // ── Nouns ────────────────────────────────────────────────────────
        Command::Service(ServiceCmd::List(a)) => cmd::service::list(a).await,
        Command::Service(ServiceCmd::Show(a)) => cmd::service::show(a).await,
        Command::Iface(IfaceCmd::List(a)) => cmd::iface::list(a).await,
        Command::Iface(IfaceCmd::Show(a)) => cmd::iface::show(a).await,
        Command::Namespace(NamespaceCmd::List(a)) => cmd::namespace::list(a).await,
        Command::Config(ConfigCmd::Get(a)) => cmd::config::get(a).await,
        Command::Config(ConfigCmd::Set(a)) => cmd::config::set(a).await,
        Command::Config(ConfigCmd::Confirm(a)) => cmd::config::confirm(a).await,
        Command::Config(ConfigCmd::Cancel(a)) => cmd::config::cancel(a).await,
        Command::Config(ConfigCmd::Extend(a)) => cmd::config::extend(a).await,
        Command::Config(ConfigCmd::Persist(a)) => cmd::config::persist(a).await,
        Command::Schema(SchemaCmd::Show(a)) => cmd::schema::show(a).await,
        Command::Storage(StorageCmd::List(a)) => cmd::storage::list(a).await,
        Command::Storage(StorageCmd::Gen(a)) => cmd::storage::plan(a).await,
        Command::Acl(AclCmd::Gen(a)) => cmd::acl::run(a).await,
        Command::Blob(BlobCmd::List(a)) => cmd::blob::list(a).await,
        Command::Blob(BlobCmd::Locate { target, bus }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::blob::locate(&target, &bus).await
        }
        Command::Blob(BlobCmd::Fetch(a)) => cmd::blob::fetch(a).await,
        Command::Admin(AdminCmd::Routers { bus }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::admin::routers(&bus).await
        }
        Command::Admin(AdminCmd::Graph(a)) => cmd::admin::graph(a).await,
        Command::Key(KeyCmd::Includes(a)) => cmd::key::includes(a),
        Command::Key(KeyCmd::Intersects(a)) => cmd::key::intersects(a),
        Command::Key(KeyCmd::Canon { expr, out }) => cmd::key::canon(&expr, out.format, out.color),
        Command::Bench(BenchCmd::Rpc(a)) => cmd::bench::rpc(a).await,

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
        Command::Export(a) => cmd::export::run(a).await,
        Command::Serve(a) => cmd::serve::run(a).await,
        Command::Gen(a) => cmd::generate::run(a, gen_target_typed).await,
        Command::Scout(a) => cmd::scout::run(a).await,

        // ── Judgement ────────────────────────────────────────────────────
        Command::Check(CheckCmd::Expect(a)) => cmd::expect::run(a).await,
        Command::Check(CheckCmd::Cutover(a)) => cmd::cutover::run(a).await,
        Command::Check(CheckCmd::Conform(a)) => cmd::conform::run(a).await,
        Command::Check(CheckCmd::Retired { for_secs, bus }) => {
            let bus = cmd::retired::ASKING.ask(Bus::resolve(&bus));
            cmd::retired::run(for_secs, &bus).await
        }
        Command::Check(CheckCmd::Probe(a)) => cmd::probe::run(a).await,
        Command::Check(CheckCmd::Schema(a)) => cmd::schema::check(a).await,
        Command::Doctor(a) => cmd::doctor::run(a).await,
        Command::Why(a) => cmd::why::run(a).await,
        Command::Watchdog(a) => cmd::watchdog::run(a).await,

        // ── Meta ─────────────────────────────────────────────────────────
        Command::Context(cmd) => context::dispatch(cmd),
        Command::Cache(cmd) => cmd::cache::dispatch(cmd).await,
        Command::Completions { shell, static_only } => completion::emit(shell, static_only),
    }
}

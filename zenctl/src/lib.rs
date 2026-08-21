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
//! * **`--registry <dir>`** (repeatable): local `registry/*.toml` files — what
//!   a checked-out application *declares*. Works with the fleet down.
//!
//! The gap between the two is drift, and `doctor` (bus + `--registry`) is the
//! command that reports it.
pub(crate) mod bus;
pub mod cli;
pub(crate) mod degrade;
pub mod errors;
pub mod input;
pub mod render;
pub mod report;
pub(crate) mod resolve;

mod cmd;
mod completion;
mod context;

use anyhow::Result;

/// `cmd/*` runs against the **resolved** flags, never the parsed ones: the
/// context is read once, at the edge, and a command that gets a [`Bus`] is
/// past every way resolution can fail (#209).
pub(crate) use crate::bus::Bus;
use crate::cli::{
    AdminCmd, BaseCmd, BenchCmd, BlobCmd, Cli, Command, InterfaceCmd, KeyCmd, NodeCmd, PubSource,
    RegistryCmd, SchemaCmd, ServiceCmd, StorageCmd, TopicCmd,
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
        Command::Topic(TopicCmd::List {
            producer,
            class,
            r#type,
            deprecated,
            watch,
            bus,
        }) => {
            let bus = Bus::resolve(&bus)?;
            let filter = cmd::watch::TopicFilter {
                producer,
                class,
                type_name: r#type,
                deprecated,
            };
            if let Some(secs) = watch {
                return cmd::watch::topic_list(secs, &filter, &bus).await;
            }
            let report = filter.apply(&bus.slice_set().await?)?;
            crate::render::emit_with(&mut std::io::stdout(), &report, bus.format(), bus.color())
        }
        Command::Topic(TopicCmd::Info { key, bus }) => {
            let bus = Bus::resolve(&bus)?;
            let report = bus.slice_set().await?.topic_info(bus.base(), &key);
            crate::render::emit_with(&mut std::io::stdout(), &report, bus.format(), bus.color())
        }
        Command::Topic(TopicCmd::Echo {
            selector,
            origin,
            class,
            producer,
            fmt,
            raw,
            hex,
            rate,
            no_decode,
            count,
            seed,
            bus,
        }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::echo::run(
                selector.as_deref(),
                origin.as_deref(),
                class.as_deref(),
                producer.as_deref(),
                fmt.as_deref(),
                raw,
                hex,
                rate,
                no_decode,
                count,
                seed,
                &bus,
            )
            .await
        }
        Command::Topic(TopicCmd::Pub {
            key,
            body,
            from,
            i_know,
            qos,
            encoding,
            repeat,
            interval,
            no_validate,
            raw,
            attachment,
            bus,
        }) => {
            let bus = Bus::resolve(&bus)?;
            // The `ArgGroup` on the variant has already refused every other
            // combination: exactly one of --from/<KEY>, and <KEY> requires
            // <BODY>. What is left is the two real shapes (#209).
            match (from, key, body) {
                (Some(PubSource::Ndjson), _, _) => {
                    cmd::publish::run_from_ndjson(qos.as_deref(), interval, i_know, &bus).await
                }
                (None, Some(key), Some(body)) => {
                    cmd::publish::run(
                        &key,
                        &body,
                        qos.as_deref(),
                        encoding.as_deref(),
                        repeat,
                        interval,
                        no_validate,
                        raw,
                        attachment.as_ref(),
                        &bus,
                    )
                    .await
                }
                (None, _, _) => unreachable!(
                    "the `source` ArgGroup requires --from or <KEY>, and <KEY> requires <BODY>"
                ),
            }
        }
        Command::Topic(TopicCmd::Retire {
            key,
            qos,
            i_know,
            bus,
        }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::publish::retire(&key, &qos, i_know, &bus).await
        }
        Command::Topic(TopicCmd::Hz {
            selector,
            origin,
            class,
            producer,
            window,
            per_key,
            loss,
            latency,
            bus,
        }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::rate::run(
                selector.as_deref(),
                origin.as_deref(),
                class.as_deref(),
                producer.as_deref(),
                window,
                per_key || latency,
                loss,
                latency,
                false,
                &bus,
            )
            .await
        }
        Command::Topic(TopicCmd::Bw {
            selector,
            origin,
            class,
            producer,
            window,
            per_key,
            bus,
        }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::rate::run(
                selector.as_deref(),
                origin.as_deref(),
                class.as_deref(),
                producer.as_deref(),
                window,
                per_key,
                false,
                false,
                true,
                &bus,
            )
            .await
        }
        Command::Node(NodeCmd::Info { origin, bus }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::node::info(&origin, &bus).await
        }
        Command::Node(NodeCmd::List {
            verbose,
            watch,
            bus,
        }) => {
            let bus = Bus::resolve(&bus)?;
            if watch {
                return cmd::node::watch(verbose, &bus).await;
            }
            cmd::node::list(verbose, &bus).await
        }
        Command::Base(BaseCmd::List { watch, bus }) => {
            let bus = Bus::resolve(&bus)?;
            if let Some(secs) = watch {
                return cmd::watch::base_list(secs, &bus).await;
            }
            cmd::base::list(&bus).await
        }
        Command::Storage(StorageCmd::List { watch, bus }) => {
            let bus = Bus::resolve(&bus)?;
            if let Some(secs) = watch {
                return cmd::watch::storage_list(secs, &bus).await;
            }
            cmd::storage::list(&bus).await
        }
        Command::Blob(BlobCmd::List {
            producer,
            tier,
            bus,
        }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::blob::list(producer.as_deref(), tier.as_deref(), &bus).await
        }
        Command::Blob(BlobCmd::Probe { target, bus }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::blob::probe(&target, &bus).await
        }
        Command::Blob(BlobCmd::Fetch {
            target,
            from,
            out,
            root,
            allow_unpinned,
            overwrite,
            quiet,
            bus,
        }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::blob::fetch(
                &target,
                &from,
                out.as_deref(),
                root.as_deref(),
                allow_unpinned,
                overwrite,
                quiet,
                &bus,
            )
            .await
        }
        Command::Admin(AdminCmd::Routers { bus }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::admin::routers(&bus).await
        }
        Command::Admin(AdminCmd::Graph { dot, origins, bus }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::admin::graph(dot, origins, &bus).await
        }
        Command::Get {
            selector,
            body,
            raw,
            hex,
            fmt,
            no_decode,
            bus,
        } => {
            let bus = Bus::resolve(&bus)?;
            cmd::get::run(
                &selector,
                body.as_ref(),
                raw,
                hex,
                fmt.as_deref(),
                no_decode,
                &bus,
            )
            .await
        }
        Command::Service(ServiceCmd::List { producer, bus }) => {
            let bus = Bus::resolve(&bus)?;
            let report = bus.slice_set().await?.service_list(producer.as_deref());
            crate::render::emit_with(&mut std::io::stdout(), &report, bus.format(), bus.color())
        }
        Command::Service(ServiceCmd::Info {
            producer,
            procedure,
            bus,
        }) => {
            let bus = Bus::resolve(&bus)?;
            let report = bus
                .slice_set()
                .await?
                .service_info(&producer, procedure.as_deref())?;
            crate::render::emit_with(&mut std::io::stdout(), &report, bus.format(), bus.color())
        }
        Command::Service(ServiceCmd::Call {
            origin,
            producer,
            procedure,
            params,
            body,
            attachment,
            no_validate,
            raw,
            bus,
        }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::call::run(
                &origin,
                &producer,
                &procedure,
                &params,
                body.as_ref(),
                attachment.as_ref(),
                no_validate,
                raw,
                &bus,
            )
            .await
        }
        Command::Interface(InterfaceCmd::List { bus }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::interface::list(&bus).await
        }
        Command::Interface(InterfaceCmd::Show {
            type_name,
            schema,
            full,
            bus,
        }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::interface::show(&type_name, schema, full, &bus).await
        }
        Command::Schema {
            cmd:
                Some(SchemaCmd::Check {
                    type_name,
                    from,
                    producer,
                    schema_set,
                    encoding,
                    bus,
                }),
            ..
        } => {
            let bus = Bus::resolve(&bus)?;
            cmd::schema::check(
                &type_name,
                &from,
                producer.as_deref(),
                schema_set.as_deref(),
                encoding.as_deref(),
                &bus,
            )
            .await
        }
        Command::Schema {
            cmd: None,
            producer: Some(producer),
            type_name,
            full,
            bus,
        } => {
            let bus = Bus::resolve(&bus)?;
            cmd::schema::dump(&producer, type_name.as_deref(), full, &bus).await
        }
        Command::Schema {
            cmd: None,
            producer: None,
            ..
        } => Err(anyhow::anyhow!(
            "schema needs a <PRODUCER> to dump, or the `check` subcommand"
        )),
        Command::Registry(RegistryCmd::Export {
            target,
            producer,
            bus,
        }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::registry::export(target, producer.as_deref(), &bus).await
        }
        Command::Registry(RegistryCmd::Diff { bus }) => {
            let bus = Bus::resolve(&bus)?;
            cmd::registry::diff(&bus).await
        }
        Command::Registry(RegistryCmd::Lint { dir, ledger, out }) => {
            cmd::registry::lint(&dir, ledger.as_ref(), out)
        }
        Command::Registry(RegistryCmd::Lock { dir, force, out }) => {
            cmd::registry::lock(&dir, force, out)
        }
        Command::Bench(BenchCmd::Rpc {
            origin,
            producer,
            procedure,
            count,
            concurrency,
            i_know,
            bus,
        }) => {
            let bus = Bus::resolve(&bus)?;
            eprintln!("{}", cmd::bench::note(bus.timeout()));
            cmd::bench::rpc(
                &origin,
                &producer,
                &procedure,
                count,
                concurrency,
                i_know,
                &bus,
            )
            .await
        }
        Command::Cache(cmd) => cmd::cache::dispatch(cmd).await,
        Command::Context(cmd) => context::dispatch(cmd),
        Command::Completions { shell, static_only } => completion::emit(shell, static_only),
        Command::Doctor {
            deep,
            sample,
            listen,
            fail_on,
            bus,
        } => {
            let bus = Bus::resolve(&bus)?;
            cmd::doctor::run(deep, sample, listen, fail_on, &bus).await
        }
        Command::Serve {
            keyexpr,
            reply,
            encoding,
            no_validate,
            raw,
            complete,
            count,
            bus,
        } => {
            let bus = Bus::resolve(&bus)?;
            cmd::serve::run(
                &keyexpr,
                &reply,
                encoding.as_deref(),
                no_validate,
                raw,
                complete,
                count,
                &bus,
            )
            .await
        }
        Command::Key(KeyCmd::Includes { a, b, out }) => {
            cmd::key::relate("includes", &a, &b, out.format, out.color)
        }
        Command::Key(KeyCmd::Intersects { a, b, out }) => {
            cmd::key::relate("intersects", &a, &b, out.format, out.color)
        }
        Command::Key(KeyCmd::Canon { expr, out }) => cmd::key::canon(&expr, out.format, out.color),
        Command::Cutover {
            old_root,
            window,
            bus,
        } => {
            let bus = Bus::resolve(&bus)?;
            cmd::cutover::run(&old_root, window, &bus).await
        }
        Command::Gen {
            producer,
            subject,
            vars,
            origin,
            rate,
            pattern,
            duration,
            seed,
            schema_set,
            serve_describe,
            dry_run,
            i_know,
            bus,
        } => {
            let bus = Bus::resolve(&bus)?;
            cmd::generate::run(
                producer.as_deref(),
                subject.as_deref(),
                &vars,
                origin.as_deref(),
                rate,
                pattern,
                duration,
                seed,
                schema_set.as_deref(),
                serve_describe,
                dry_run,
                i_know,
                &bus,
            )
            .await
        }
        Command::Expect {
            selector,
            within,
            count,
            rate_min,
            rate_max,
            valid_payload,
            qos,
            absent,
            bus,
        } => {
            let bus = Bus::resolve(&bus)?;
            cmd::expect::run(
                &selector,
                within,
                count,
                rate_min,
                rate_max,
                valid_payload,
                qos.as_deref(),
                absent,
                &bus,
            )
            .await
        }
        Command::Probe {
            target,
            producer,
            procedure,
            bus,
        } => {
            let bus = Bus::resolve(&bus)?;
            cmd::probe::run(&target, &producer, &procedure, &bus).await
        }
        Command::Record {
            selector,
            origin,
            class,
            producer,
            out,
            duration,
            count,
            bus,
        } => {
            let bus = Bus::resolve(&bus)?;
            cmd::record::run(
                selector.as_deref(),
                origin.as_deref(),
                class.as_deref(),
                producer.as_deref(),
                &out,
                duration,
                count,
                &bus,
            )
            .await
        }
        Command::Replay {
            file,
            speed,
            dry_run,
            force_base,
            i_know,
            qos,
            bus,
        } => {
            let bus = Bus::resolve(&bus)?;
            cmd::replay::run(&file, speed, dry_run, force_base, i_know, &qos, &bus).await
        }
        Command::Scout {
            what,
            timeout,
            connect,
            listen,
            context,
            out,
        } => {
            // No BusArgs here: --base/--registry are meaningless before a
            // session exists. Contexts still resolve, for endpoints/timeout —
            // and this is the caller that proves the ladders must take
            // scalars: when they demanded a `BusArgs`, this verb grew its own
            // copy of two of them instead (#209).
            // A bad name is an `Err` here too, and for the same reason it
            // is one in `Bus::resolve`: this was the *second* `exit(2)` for
            // one failure, and two exit codes for one mistake is how a
            // contract stops being a contract.
            let stored = context::active(context.as_deref())?;
            let stored = stored.as_ref();
            let connect = resolve::endpoints(&connect, stored.map(|c| c.connect.as_slice()));
            let listen = resolve::endpoints(&listen, stored.map(|c| c.listen.as_slice()));
            let timeout = resolve::timeout(timeout, stored);
            cmd::scout::run(&what, timeout, &connect, &listen, out.format, out.color).await
        }
    }
}

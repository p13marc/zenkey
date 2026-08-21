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
pub mod cli;
pub mod errors;
pub mod render;
pub mod report;

mod cmd;
mod completion;
mod context;

use anyhow::Result;
use clap::Parser as _;

use std::time::Duration;

/// `cmd/*` reaches for `crate::BusArgs`; #209 moves it to `cli/bus.rs` and
/// the ladders out of it entirely, so the re-export keeps twenty call sites
/// out of that churn.
pub(crate) use crate::cli::BusArgs;
use crate::cli::{
    AdminCmd, BaseCmd, BenchCmd, BlobCmd, CacheCmd, Cli, Command, ContextCmd, InterfaceCmd, KeyCmd,
    NodeCmd, PubSource, RegistryCmd, SchemaCmd, ServiceCmd, StorageCmd, TopicCmd,
};
use zenkey_fleet as bus;

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

    let cli = Cli::parse();
    match cli.command {
        Command::Topic(TopicCmd::List {
            producer,
            class,
            r#type,
            deprecated,
            watch,
            bus,
        }) => {
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
        }) => match (from, key, body) {
            (Some(PubSource::Ndjson), None, None) => {
                cmd::publish::run_from_ndjson(qos.as_deref(), interval, i_know, &bus).await
            }
            (Some(_), _, _) => Err(anyhow::anyhow!(
                "--from ndjson reads keys and payloads from stdin rows — drop the \
                 key/body arguments"
            )),
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
                    attachment.as_deref(),
                    &bus,
                )
                .await
            }
            (None, _, _) => Err(anyhow::anyhow!(
                "topic pub needs <KEY> <BODY>, or --from ndjson with rows on stdin"
            )),
        },
        Command::Topic(TopicCmd::Retire {
            key,
            qos,
            i_know,
            bus,
        }) => cmd::publish::retire(&key, &qos, i_know, &bus).await,
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
        Command::Node(NodeCmd::Info { origin, bus }) => cmd::node::info(&origin, &bus).await,
        Command::Node(NodeCmd::List {
            verbose,
            watch,
            bus,
        }) => {
            if watch {
                return cmd::node::watch(verbose, &bus).await;
            }
            cmd::node::list(verbose, &bus).await
        }
        Command::Base(BaseCmd::List { watch, bus }) => {
            if let Some(secs) = watch {
                return cmd::watch::base_list(secs, &bus).await;
            }
            // Deliberately never calls bus.base() — this is the command that
            // answers "what would I even pass as --base?".
            let session = bus.session().await?;
            let bases = bus::discover_bases(&session, bus.timeout()).await?;
            crate::render::emit_with(
                &mut std::io::stdout(),
                &report::BaseList { bases },
                bus.format(),
                bus.color(),
            )
        }
        Command::Storage(StorageCmd::List { watch, bus }) => {
            if let Some(secs) = watch {
                return cmd::watch::storage_list(secs, &bus).await;
            }
            let session = bus.session().await?;
            let storages = zenkey_fleet::storages(&session, bus.timeout()).await?;
            // The coverage join needs slices; degrade to storages-only when
            // none resolve (fleet down and no --registry).
            let coverage = match bus.slice_set().await {
                Ok(slices) => zenkey_fleet::state_coverage(&slices, bus.base(), &storages),
                Err(e) => {
                    eprintln!("note: no slices for the coverage join ({e})");
                    Vec::new()
                }
            };
            crate::render::emit_with(
                &mut std::io::stdout(),
                &report::StorageList { storages, coverage },
                bus.format(),
                bus.color(),
            )
        }
        Command::Blob(BlobCmd::List {
            producer,
            tier,
            bus,
        }) => cmd::blob::list(producer.as_deref(), tier.as_deref(), &bus).await,
        Command::Blob(BlobCmd::Probe { target, bus }) => cmd::blob::probe(&target, &bus).await,
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
        Command::Admin(AdminCmd::Routers { bus }) => cmd::admin::routers(&bus).await,
        Command::Admin(AdminCmd::Graph { dot, origins, bus }) => {
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
            cmd::get::run(
                &selector,
                body.as_deref(),
                raw,
                hex,
                fmt.as_deref(),
                no_decode,
                &bus,
            )
            .await
        }
        Command::Service(ServiceCmd::List { producer, bus }) => {
            let report = bus.slice_set().await?.service_list(producer.as_deref());
            crate::render::emit_with(&mut std::io::stdout(), &report, bus.format(), bus.color())
        }
        Command::Service(ServiceCmd::Info {
            producer,
            procedure,
            bus,
        }) => {
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
            cmd::call::run(
                &origin,
                &producer,
                &procedure,
                &params,
                body.as_deref(),
                attachment.as_deref(),
                no_validate,
                raw,
                &bus,
            )
            .await
        }
        Command::Interface(InterfaceCmd::List { bus }) => {
            let report = bus.slice_set().await?.interface_list();
            crate::render::emit_with(&mut std::io::stdout(), &report, bus.format(), bus.color())
        }
        Command::Interface(InterfaceCmd::Show {
            type_name,
            schema,
            full,
            bus,
        }) => {
            let slices = bus.slices().await?;
            let mut report =
                zenkey_fleet::SliceSet::from_slices(slices.clone()).interface_show(&type_name)?;
            if schema {
                // Only the producers that carry the type are asked — the
                // registry already says who, so this is never a fleet fan-out.
                let producers = cmd::schema::carriers_of(&slices, &type_name);
                let session = bus.session().await?;
                let store = zenkey_fleet::decode::SchemaStore::new(bus.base(), bus.timeout());
                report.schemas =
                    zenkey_fleet::schemas_for_type(&store, &session, &producers, &type_name, full)
                        .await;
            }
            crate::render::emit_with(&mut std::io::stdout(), &report, bus.format(), bus.color())
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
        } => cmd::schema::dump(&producer, type_name.as_deref(), full, &bus).await,
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
        }) => cmd::registry::export(target, producer.as_deref(), &bus).await,
        Command::Registry(RegistryCmd::Diff { bus }) => cmd::registry::diff(&bus).await,
        Command::Registry(RegistryCmd::Lint { dir, ledger }) => {
            cmd::registry::lint(&dir, ledger.as_ref())
        }
        Command::Registry(RegistryCmd::Lock { dir, force }) => cmd::registry::lock(&dir, force),
        Command::Bench(BenchCmd::Rpc {
            origin,
            producer,
            procedure,
            count,
            concurrency,
            i_know,
            bus,
        }) => {
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
        Command::Cache(CacheCmd::Show { bus }) => cmd::cache::show(&bus),
        Command::Cache(CacheCmd::Refresh { bus }) => cmd::cache::refresh(&bus).await,
        Command::Cache(CacheCmd::Clear { bus }) => cmd::cache::clear(&bus),
        Command::Context(cmd) => match cmd {
            ContextCmd::Create {
                name,
                base,
                connect,
                listen,
                registry,
                scouting,
                timeout,
                zenoh_config,
                select,
            } => context::create(
                &name,
                context::StoredContext {
                    base,
                    connect,
                    listen,
                    registry,
                    scouting: scouting.then_some(true),
                    timeout,
                    zenoh_config,
                },
                select,
            ),
            ContextCmd::List => context::list(),
            ContextCmd::Edit => context::edit(),
            ContextCmd::Show { name } => context::show(name.as_deref()),
            ContextCmd::Select { name } => context::select(&name),
            ContextCmd::Rm { name } => context::remove(&name),
        },
        Command::Completions { shell, static_only } => {
            use clap::CommandFactory as _;
            if static_only {
                clap_complete::aot::generate(
                    shell,
                    &mut Cli::command(),
                    "zenctl",
                    &mut std::io::stdout(),
                );
                return Ok(());
            }
            completion::registration(shell)
        }
        Command::Doctor {
            deep,
            sample,
            listen,
            fail_on,
            bus,
        } => cmd::doctor::run(deep, sample, listen, fail_on, &bus).await,
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
        } => cmd::cutover::run(&old_root, window, &bus).await,
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
        } => cmd::probe::run(&target, &producer, &procedure, &bus).await,
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
        } => cmd::replay::run(&file, speed, dry_run, force_base, i_know, &qos, &bus).await,
        Command::Scout {
            what,
            timeout,
            connect,
            listen,
            context,
            out,
        } => {
            // No BusArgs here: --base/--registry are meaningless before a
            // session exists. Contexts still resolve, for endpoints/timeout.
            let stored = match context::active(context.as_deref()) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(2);
                }
            };
            let connect = if connect.is_empty() {
                stored
                    .as_ref()
                    .map(|c| c.connect.clone())
                    .unwrap_or_default()
            } else {
                connect
            };
            let listen = if listen.is_empty() {
                stored
                    .as_ref()
                    .map(|c| c.listen.clone())
                    .unwrap_or_default()
            } else {
                listen
            };
            let timeout = Duration::from_secs(
                timeout
                    .or_else(|| stored.as_ref().and_then(|c| c.timeout))
                    .unwrap_or(5),
            );
            cmd::scout::run(&what, timeout, &connect, &listen, out.format, out.color).await
        }
    }
}

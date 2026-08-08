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

mod context;
mod offline;
mod output;
mod report;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, anyhow};
use clap::{Args, Parser, Subcommand};
use zenkey::RegistrySlice;
use zenkey_fleet as bus;

#[derive(Parser)]
#[command(
    name = "zenctl",
    about = "Explore a keyspace-v2 Zenoh bus (RFC 08 §6)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Subjects: what data exists, and what it means.
    #[command(subcommand)]
    Topic(TopicCmd),
    /// Producers: who is alive on the bus.
    #[command(subcommand)]
    Node(NodeCmd),
    /// Deployment bases discovered from the wire (needs no --base).
    #[command(subcommand)]
    Base(BaseCmd),
    /// Procedures on the `@rpc` plane.
    #[command(subcommand)]
    Service(ServiceCmd),
    /// Payload types declared by the registry slices.
    #[command(subcommand)]
    Interface(InterfaceCmd),
    /// Zenoh admin space (`@/**`) — the middleware's own introspection.
    #[command(subcommand)]
    Admin(AdminCmd),
    /// Storages: what the mesh persists, joined against declared state.
    #[command(subcommand)]
    Storage(StorageCmd),
    /// Manage named connection contexts (config file).
    #[command(subcommand)]
    Context(ContextCmd),
    /// Generate shell completions (bash, zsh, fish, elvish, powershell).
    ///
    /// e.g. `zenctl completions bash > ~/.local/share/bash-completion/completions/zenctl`
    Completions { shell: clap_complete::Shell },
    /// Diff what the fleet *serves* against local registry files.
    ///
    /// RFC 08 §6: "A disagreement between introspection and the checked-in TOML
    /// is a finding, not an ambiguity." This prints the findings. The local
    /// truth comes from `--registry <dir>`; without it only the
    /// roster-vs-introspect check runs.
    Doctor {
        /// Additionally GET current state to check freshness against each
        /// subject's ttl (RFC 04 §1.2) and judge storage coverage — adds
        /// fleet query load.
        #[arg(long)]
        deep: bool,
        #[command(flatten)]
        bus: BusArgs,
    },
}

#[derive(Subcommand)]
enum AdminCmd {
    /// GET an admin selector and print key/value pairs.
    ///
    /// Admin key layouts vary between zenoh versions — this is a raw,
    /// honest browse. Requires an un-namespaced session (which zenctl
    /// always runs, RFC 09 §5).
    Get {
        /// Admin selector.
        #[arg(default_value = "@/**")]
        selector: String,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Enumerate routers/peers (zid, version, locators).
    Routers {
        #[command(flatten)]
        bus: BusArgs,
    },
}

#[derive(Subcommand)]
enum StorageCmd {
    /// List configured storages and judge declared state families against
    /// them (covered / partial / uncovered — RFC 04 §4, issue #14).
    List {
        #[command(flatten)]
        bus: BusArgs,
    },
}

#[derive(Subcommand)]
enum ContextCmd {
    /// Create (or update) a named context.
    Create {
        name: String,
        /// The deployment base this context pins.
        #[arg(long)]
        base: Option<String>,
        /// Endpoint to connect to, repeatable.
        #[arg(long, short = 'c')]
        connect: Vec<String>,
        /// Endpoint to listen on, repeatable.
        #[arg(long, short = 'l')]
        listen: Vec<String>,
        /// Registry dir, repeatable (offline slice source).
        #[arg(long, value_name = "DIR")]
        registry: Vec<PathBuf>,
        /// Enable multicast scouting for this context.
        #[arg(long)]
        scouting: bool,
        /// Default reply timeout in seconds.
        #[arg(long)]
        timeout: Option<u64>,
        /// Select it as the current context.
        #[arg(long)]
        select: bool,
    },
    /// List contexts (the `*` marks the current one).
    List,
    /// Show one context (default: the current one).
    Show { name: Option<String> },
    /// Select the current context.
    Select { name: String },
    /// Remove a context.
    Rm { name: String },
}

#[derive(Subcommand)]
enum TopicCmd {
    /// List registered subjects.
    ///
    /// Reads each producer's served introspect slice off the live bus by
    /// default — so it works against *any* keyspace-v2 fleet (RFC 08 §6).
    /// With `--registry <dir>` it answers offline from local registry TOMLs.
    List {
        /// Only this producer.
        #[arg(long)]
        producer: Option<String>,
        /// Only this class: telemetry, state, or events.
        #[arg(long)]
        class: Option<String>,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Describe one key or subject pattern.
    ///
    /// Accepts a full wire key (`<base>/v1/h-abc.../telemetry/sysinfo/cpu/usage`)
    /// and refines it against the producer's registry slice (bus-served, or
    /// local with `--registry`).
    Info {
        /// A concrete wire key, as it appears on the bus.
        key: String,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Subscribe and print decoded samples (on-bus).
    ///
    /// With a served `describe` schema (RFC 08 §7) payloads decode into
    /// named fields; otherwise they render structurally, tagged with the
    /// registry-declared type. --origin/--class/--producer compose the
    /// selector server-side — never client-side filtering the grammar can
    /// express by position.
    Echo {
        /// Explicit key expression (overrides --origin/--class/--producer).
        /// Defaults to all v1 data under the base: `<base>/v1/**`.
        selector: Option<String>,
        /// Only this origin (`h-…` or `@service`).
        #[arg(long)]
        origin: Option<String>,
        /// Only this class: telemetry, state, or events.
        #[arg(long)]
        class: Option<String>,
        /// Only this producer.
        #[arg(long)]
        producer: Option<String>,
        /// kcat-style format string: %k wire key, %K base-relative, %o origin,
        /// %c class, %p producer, %s subject, %t type, %v value, %e encoding,
        /// %l payload bytes, %n counter, %T timestamp, %{a.b.c} a decoded
        /// payload field by dot-path, %% literal percent.
        #[arg(long)]
        fmt: Option<String>,
        /// Print raw payload bytes as hex instead of decoding.
        #[arg(long)]
        raw: bool,
        /// Decode the type tag but show the payload as hex.
        #[arg(long)]
        hex: bool,
        /// Append the live aggregate sample rate to each line.
        #[arg(long)]
        rate: bool,
        /// Skip schema decode (structural rendering only).
        #[arg(long)]
        no_decode: bool,
        /// Stop after this many samples (0 = run until interrupted).
        #[arg(long, default_value_t = 0)]
        count: usize,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Publish to a key (issue #47) — a declared publisher, never an ad-hoc
    /// put (P7).
    Pub {
        /// Full wire key to publish on.
        key: String,
        /// Payload: inline text, `@file`, or `-` for stdin.
        body: String,
        /// QoS profile (RFC 04 §3): sampled|refreshed|transition|alert|frame.
        #[arg(long, default_value = "sampled")]
        qos: String,
        /// Wire encoding to declare (e.g. application/json). Defaults to the
        /// registry's declared encoding when the key refines, else none.
        #[arg(long)]
        encoding: Option<String>,
        /// Publish this many times (0 = once).
        #[arg(long, default_value_t = 0)]
        repeat: usize,
        /// Seconds between repeats.
        #[arg(long, default_value_t = 1.0)]
        interval: f64,
        /// Skip registry/schema validation of the body.
        #[arg(long)]
        no_validate: bool,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Measure publish rate over a window (ros2-style).
    Hz {
        /// Key expression (or use --origin/--class/--producer composition).
        selector: Option<String>,
        #[arg(long)]
        origin: Option<String>,
        #[arg(long)]
        class: Option<String>,
        #[arg(long)]
        producer: Option<String>,
        /// Measurement window, seconds.
        #[arg(long, default_value_t = 10)]
        window: u64,
        /// Report each concrete key separately.
        #[arg(long)]
        per_key: bool,
        /// Also report source-sequence gaps (needs publishers that attach
        /// SourceInfo; absent info reads as zero, honestly labeled).
        #[arg(long)]
        loss: bool,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Measure payload bandwidth over a window.
    Bw {
        /// Key expression (or use --origin/--class/--producer composition).
        selector: Option<String>,
        #[arg(long)]
        origin: Option<String>,
        #[arg(long)]
        class: Option<String>,
        #[arg(long)]
        producer: Option<String>,
        /// Measurement window, seconds.
        #[arg(long, default_value_t = 10)]
        window: u64,
        /// Report each concrete key separately.
        #[arg(long)]
        per_key: bool,
        #[command(flatten)]
        bus: BusArgs,
    },
}

#[derive(Subcommand)]
enum NodeCmd {
    /// List live producers from the liveliness roster (on-bus).
    List {
        /// Join each producer against its served introspect slice (app +
        /// registry version).
        #[arg(long)]
        verbose: bool,
        #[command(flatten)]
        bus: BusArgs,
    },
}

#[derive(Subcommand)]
enum BaseCmd {
    /// Sweep liveliness tokens and storage configs for the bases in use.
    ///
    /// The command to run *before* you have a base: the un-namespaced sweep
    /// (`**/v1/*/state/*/alive`, plus `@catalog` by name and the router
    /// storage configs) attributes every alive token to its base. An empty
    /// base (keys start at `v1/` on the wire) is reported as `(empty)` and
    /// selected with `--base ""`.
    List {
        #[command(flatten)]
        bus: BusArgs,
    },
}

#[derive(Subcommand)]
enum ServiceCmd {
    /// List registered procedures (bus-served slices, or `--registry`).
    List {
        /// Only this producer.
        #[arg(long)]
        producer: Option<String>,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Call a procedure (on-bus).
    Call {
        /// Origin to target: a host id (`h-3fa9c2d41b7e`), `*` for the whole
        /// fleet, or `@catalog` for a service origin.
        origin: String,
        /// Producer name. Omit for a service origin, which has no producer chunk.
        producer: String,
        /// Procedure path, e.g. `introspect` or `artifact/status`.
        procedure: String,
        /// Selector parameters, repeatable: `--param state=established`.
        #[arg(long = "param", value_name = "K=V")]
        params: Vec<String>,
        /// Request body: inline JSON, or `@path` to read a file.
        #[arg(long)]
        body: Option<String>,
        /// Skip the registry lookup (and with it the registry-layer
        /// forbidden-fanout refusal and any body validation).
        #[arg(long)]
        no_validate: bool,
        #[command(flatten)]
        bus: BusArgs,
    },
}

#[derive(Subcommand)]
enum InterfaceCmd {
    /// List every payload type the registry slices declare.
    List {
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Show one payload type and every subject that carries it.
    Show {
        /// Type name, e.g. `TelemetryPoint`.
        type_name: String,
        #[command(flatten)]
        bus: BusArgs,
    },
}

/// Options shared by every command: the deployment base, the registry source,
/// and the connection.
#[derive(Args, Clone)]
struct BusArgs {
    /// The deployment base — the first chunk(s) of every key on the wire.
    ///
    /// Applications set this as their Zenoh session `namespace` and never
    /// spell it. `zenctl` deliberately does **not**: a debug tool runs
    /// un-namespaced so it sees the wire as it really is, including traffic
    /// from outside the deployment (RFC 09 §5) — which is what lets it spot a
    /// leak. So it has to be told what the base is.
    /// Resolution: flag > env > active context (`zenctl context …`) > empty —
    /// the base-less bus-root deployment, the RFC v1.6 default, whose wire
    /// keys start at `v1/`. `zenctl base list` discovers the bases in use.
    #[arg(long, env = "ZENCTL_BASE")]
    base: Option<String>,
    /// Use a named context from the config file for this invocation
    /// (default: the file's `current` pointer; env `ZENCTL_CONTEXT`).
    #[arg(long, value_name = "NAME")]
    context: Option<String>,
    /// Local registry directory (`registry/*.toml`), repeatable. When given,
    /// registry-sourced commands answer offline from these files instead of
    /// the live bus.
    #[arg(long, value_name = "DIR")]
    registry: Vec<PathBuf>,
    /// Endpoint to connect to, repeatable (e.g. `tcp/127.0.0.1:7447`).
    #[arg(long, short = 'c')]
    connect: Vec<String>,
    /// Endpoint to listen on, repeatable.
    #[arg(long, short = 'l')]
    listen: Vec<String>,
    /// Enable multicast scouting.
    ///
    /// OFF by default, and you should think before turning it on: a scouting
    /// explorer joins whatever mesh it can find, which is how a throwaway
    /// session ends up talking to a production fleet.
    #[arg(long)]
    scouting: bool,
    /// Seconds to wait for replies / to watch the bus (default 5; a
    /// context may override the default).
    #[arg(long)]
    timeout: Option<u64>,
    /// Output format: table for humans, json (one document) or ndjson (one
    /// object per row) for scripts; auto = table on a tty, ndjson piped.
    #[arg(long, env = "ZENCTL_FORMAT", value_enum, default_value_t = output::Format::Auto)]
    format: output::Format,
}

impl BusArgs {
    /// The active stored context, loaded once per process (one BusArgs is
    /// ever used per invocation). A bad --context/ZENCTL_CONTEXT is fatal.
    fn stored(&self) -> Option<&'static context::StoredContext> {
        static ACTIVE: std::sync::OnceLock<Option<context::StoredContext>> =
            std::sync::OnceLock::new();
        ACTIVE
            .get_or_init(|| match context::active(self.context.as_deref()) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(2);
                }
            })
            .as_ref()
    }
    /// The deployment base: flag > env > active context > empty (the
    /// base-less bus-root deployment — the RFC v1.6 default).
    fn base(&self) -> &str {
        if let Some(b) = &self.base {
            return b.as_str();
        }
        self.stored().and_then(|c| c.base.as_deref()).unwrap_or("")
    }
    async fn session(&self) -> Result<zenoh::Session> {
        let (connect, listen);
        let stored = self.stored();
        connect = if self.connect.is_empty() {
            stored.map(|c| c.connect.clone()).unwrap_or_default()
        } else {
            self.connect.clone()
        };
        listen = if self.listen.is_empty() {
            stored.map(|c| c.listen.clone()).unwrap_or_default()
        } else {
            self.listen.clone()
        };
        let scouting = self.scouting || stored.and_then(|c| c.scouting).unwrap_or(false);
        bus::open(&connect, &listen, scouting).await
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(
            self.timeout
                .or_else(|| self.stored().and_then(|c| c.timeout))
                .unwrap_or(5),
        )
    }
    fn registry_dirs(&self) -> Vec<PathBuf> {
        if !self.registry.is_empty() {
            self.registry.clone()
        } else {
            self.stored()
                .map(|c| c.registry.clone())
                .unwrap_or_default()
        }
    }
    /// Compose a base-relative key into the full wire key this un-namespaced
    /// tool must actually use.
    fn wire(&self, relative: impl AsRef<str>) -> Result<String> {
        Ok(zenkey::grammar::with_base(self.base(), relative))
    }
    /// Registry slices from whichever source the flags select: local
    /// `--registry` dirs when given (offline), otherwise the live bus
    /// (RFC 08 §6 introspection). Both yield the same `Vec<RegistrySlice>`,
    /// so every renderer is source-agnostic.
    async fn slices(&self) -> Result<Vec<RegistrySlice>> {
        Ok(self.slice_set().await?.slices().to_vec())
    }
    /// The same, as the fleet engine's indexed set (echo's decode path).
    async fn slice_set(&self) -> Result<zenkey_fleet::SliceSet> {
        let dirs = self.registry_dirs();
        let session = self.session().await?;
        let base = self.base();
        if !dirs.is_empty() {
            // §6.1's decision, delivered by issue #43: --registry and the bus
            // stop being exclusive. Union: served wins per producer, dirs
            // fill the gaps, disagreement is reported — never silently
            // overwritten.
            let out =
                zenkey_fleet::SliceSet::from_union(&session, base, &dirs, self.timeout()).await?;
            for d in &out.disagreements {
                eprintln!(
                    "registry disagreement: {} — bus serves v{}, dirs carry v{}{}                      (served wins; `zenctl registry diff` details it)",
                    d.producer,
                    d.bus_version,
                    d.dirs_version,
                    if d.shape_differs {
                        ", shapes differ"
                    } else {
                        ""
                    }
                );
            }
            return Ok(out.set);
        }
        let set = zenkey_fleet::SliceSet::from_bus(&session, base, self.timeout()).await?;
        if set.slices().is_empty() {
            eprintln!(
                "no introspect slices on base {base:?} — an empty set is not a verdict (RFC 05 §3.1); \
                 `zenctl node list --base {base:?}` says who is actually up.\n\
                 (offline alternative: --registry <dir> with the app's registry TOMLs)"
            );
        }
        Ok(set)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Behave like a Unix filter under `zenctl … | head`: Rust masks SIGPIPE,
    // turning a closed pipe into a mid-write panic; restore the default
    // disposition so the process exits quietly (141) instead.
    #[cfg(unix)]
    // SAFETY: installing SIG_DFL (not a handler fn) is process-wide and
    // has no safety obligations beyond the FFI call itself.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

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
            bus,
        }) => {
            let slices = bus.slices().await?;
            let report = offline::topic_list(&slices, producer.as_deref(), class.as_deref())?;
            output::topic_list(&report, bus.format)
        }
        Command::Topic(TopicCmd::Info { key, bus }) => {
            let slices = bus.slices().await?;
            let report = offline::topic_info(bus.base(), &key, &slices);
            output::topic_info(&report, bus.format)
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
            bus,
        }) => {
            cmd_echo(
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
                &bus,
            )
            .await
        }
        Command::Topic(TopicCmd::Pub {
            key,
            body,
            qos,
            encoding,
            repeat,
            interval,
            no_validate,
            bus,
        }) => {
            cmd_pub(
                &key,
                &body,
                &qos,
                encoding.as_deref(),
                repeat,
                interval,
                no_validate,
                &bus,
            )
            .await
        }
        Command::Topic(TopicCmd::Hz {
            selector,
            origin,
            class,
            producer,
            window,
            per_key,
            loss,
            bus,
        }) => {
            cmd_rate(
                selector.as_deref(),
                origin.as_deref(),
                class.as_deref(),
                producer.as_deref(),
                window,
                per_key,
                loss,
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
            cmd_rate(
                selector.as_deref(),
                origin.as_deref(),
                class.as_deref(),
                producer.as_deref(),
                window,
                per_key,
                false,
                true,
                &bus,
            )
            .await
        }
        Command::Node(NodeCmd::List { verbose, bus }) => cmd_node_list(verbose, &bus).await,
        Command::Base(BaseCmd::List { bus }) => {
            // Deliberately never calls bus.base() — this is the command that
            // answers "what would I even pass as --base?".
            let session = bus.session().await?;
            let bases = bus::discover_bases(&session, bus.timeout()).await?;
            output::base_list(&report::BaseList { bases }, bus.format)
        }
        Command::Storage(StorageCmd::List { bus }) => {
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
            output::storage_list(&report::StorageList { storages, coverage }, bus.format)
        }
        Command::Admin(AdminCmd::Get { selector, bus }) => {
            let session = bus.session().await?;
            let entries = zenkey_fleet::admin_get(&session, &selector, bus.timeout()).await?;
            match bus.format.resolved() {
                output::Format::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &entries
                            .iter()
                            .map(|e| serde_json::json!({"key": e.key, "value": e.value}))
                            .collect::<Vec<_>>()
                    )?
                ),
                output::Format::Ndjson => {
                    for e in &entries {
                        println!("{}", serde_json::json!({"key": e.key, "value": e.value}));
                    }
                }
                _ => {
                    for e in &entries {
                        println!("{}\n  {}", e.key, e.value);
                    }
                    eprintln!("{} admin entr(ies)", entries.len());
                }
            }
            Ok(())
        }
        Command::Admin(AdminCmd::Routers { bus }) => {
            let session = bus.session().await?;
            let routers = zenkey_fleet::routers(&session, bus.timeout()).await?;
            match bus.format.resolved() {
                output::Format::Json => {
                    println!("{}", serde_json::to_string_pretty(&routers)?)
                }
                output::Format::Ndjson => {
                    for r in &routers {
                        println!("{}", serde_json::to_string(r)?);
                    }
                }
                _ => {
                    if routers.is_empty() {
                        println!(
                            "no routers answered @/*/router — a peer-only mesh, or the admin \
                             space is disabled."
                        );
                    }
                    for r in &routers {
                        println!(
                            "{}  {}  {}",
                            r.zid,
                            r.version.as_deref().unwrap_or("-"),
                            r.locators.join(", ")
                        );
                    }
                }
            }
            Ok(())
        }
        Command::Service(ServiceCmd::List { producer, bus }) => {
            let slices = bus.slices().await?;
            let report = offline::service_list(&slices, producer.as_deref())?;
            output::service_list(&report, bus.format)
        }
        Command::Service(ServiceCmd::Call {
            origin,
            producer,
            procedure,
            params,
            body,
            no_validate,
            bus,
        }) => {
            cmd_service_call(
                &origin,
                &producer,
                &procedure,
                &params,
                body.as_deref(),
                no_validate,
                &bus,
            )
            .await
        }
        Command::Interface(InterfaceCmd::List { bus }) => {
            let slices = bus.slices().await?;
            let report = offline::interface_list(&slices)?;
            output::interface_list(&report, bus.format)
        }
        Command::Interface(InterfaceCmd::Show { type_name, bus }) => {
            let slices = bus.slices().await?;
            let report = offline::interface_show(&slices, &type_name)?;
            output::interface_show(&report, bus.format)
        }
        Command::Context(cmd) => match cmd {
            ContextCmd::Create {
                name,
                base,
                connect,
                listen,
                registry,
                scouting,
                timeout,
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
                },
                select,
            ),
            ContextCmd::List => context::list(),
            ContextCmd::Show { name } => context::show(name.as_deref()),
            ContextCmd::Select { name } => context::select(&name),
            ContextCmd::Rm { name } => context::remove(&name),
        },
        Command::Completions { shell } => {
            use clap::CommandFactory as _;
            clap_complete::generate(shell, &mut Cli::command(), "zenctl", &mut std::io::stdout());
            Ok(())
        }
        Command::Doctor { deep, bus } => cmd_doctor(deep, &bus).await,
    }
}

/// `node list` — the liveliness roster (RFC 04 §5); `--verbose` joins each
/// producer against its served introspect slice.
async fn cmd_node_list(verbose: bool, args: &BusArgs) -> Result<()> {
    let session = args.session().await?;
    let mut roster = bus::roster(&session, args.base(), args.timeout()).await?;

    if roster.is_empty() {
        eprintln!(
            "nothing held a liveliness token on {} — check --connect (and that \
             producers are actually running).",
            zenkey::selector::all_liveliness(zenkey::selector::Scope::fleet())
        );
    }
    if verbose {
        let slices = args.slice_set().await?;
        for producers in roster.values_mut() {
            for p in producers.iter_mut() {
                // Instance suffixes share the base slice (RFC 03 §1.5).
                let base_name = zenkey::grammar::Producer::parse_chunk(p)
                    .map(|pr| pr.name().to_string())
                    .unwrap_or_else(|_| p.clone());
                if let Some(slice) = slices.get(&base_name) {
                    *p = format!("{p}  (app {}, registry {})", slice.app, slice.version);
                } else {
                    *p = format!("{p}  (no served slice)");
                }
            }
        }
    }
    output::node_list(&report::NodeList { origins: roster }, args.format)
}

/// Compose a server-side selector from origin/class/producer positions
/// (RFC 03: positions, not filters — never client-filter what the grammar
/// can say). `None` positions wildcard.
fn compose_selector(
    args: &BusArgs,
    origin: Option<&str>,
    class: Option<&str>,
    producer: Option<&str>,
) -> Result<String> {
    if let Some(c) = class
        && !["telemetry", "state", "events"].contains(&c)
    {
        return Err(anyhow!(
            "unknown class {c:?} — the classes are telemetry, state, events (RFC 04 §1)"
        ));
    }
    let origin = origin.unwrap_or("*");
    let class = class.unwrap_or("*");
    let rel = match producer {
        Some(p) => format!("v1/{origin}/{class}/{p}/**"),
        None if class == "*" => format!("v1/{origin}/**"),
        None => format!("v1/{origin}/{class}/**"),
    };
    args.wire(rel)
}

/// One decoded/rendered sample line for echo.
#[allow(clippy::too_many_arguments)]
fn format_sample(
    fmt: &str,
    n: usize,
    wire_key: &str,
    base: &str,
    type_name: Option<&str>,
    encoding: &str,
    payload_len: usize,
    timestamp: Option<&str>,
    value: &str,
) -> String {
    let parsed = zenkey::grammar::parse_full(base, wire_key);
    let mut out = String::with_capacity(fmt.len() + value.len());
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            }
            continue;
        }
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('{') => {
                // %{a.b.c}: a decoded payload field by dot-path. Unresolvable
                // paths render as an empty field, honestly (the line shape
                // stays stable for cut/awk).
                let mut path = String::new();
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    path.push(c);
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(value) {
                    let mut cur = &v;
                    let mut ok = true;
                    for seg in path.split('.') {
                        match cur.get(seg) {
                            Some(next) => cur = next,
                            None => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if ok {
                        match cur {
                            serde_json::Value::String(s) => out.push_str(s),
                            other => out.push_str(&other.to_string()),
                        }
                    }
                }
            }
            Some('k') => out.push_str(wire_key),
            Some('K') => {
                out.push_str(zenkey::grammar::strip_base(base, wire_key).unwrap_or(wire_key))
            }
            Some('o') => {
                if let Some(p) = &parsed {
                    out.push_str(p.origin.chunk());
                }
            }
            Some('c') => {
                if let Some(p) = &parsed {
                    out.push_str(match &p.class {
                        zenkey::grammar::ClassOrPlane::Class(c) => c.chunk(),
                        zenkey::grammar::ClassOrPlane::Plane(pl) => pl.chunk(),
                    });
                }
            }
            Some('p') => {
                if let Some(name) = parsed.as_ref().and_then(|p| p.producer.as_ref()) {
                    out.push_str(name.name());
                }
            }
            Some('s') => {
                if let Some(p) = &parsed {
                    out.push_str(&p.subject.join("/"));
                }
            }
            Some('t') => out.push_str(type_name.unwrap_or("unregistered")),
            Some('v') => out.push_str(value),
            Some('e') => out.push_str(encoding),
            Some('l') => {
                use std::fmt::Write as _;
                let _ = write!(out, "{payload_len}");
            }
            Some('n') => {
                use std::fmt::Write as _;
                let _ = write!(out, "{n}");
            }
            Some('T') => out.push_str(timestamp.unwrap_or("-")),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

/// `topic echo` v2 — subscribe, refine, schema-decode (RFC 08 §7) with
/// honest structural fallback.
///
/// Subscribe-first is not a style choice: RFC 04 §3.2 forbids
/// GET-then-subscribe (it drops everything published in the gap).
#[allow(clippy::too_many_arguments)]
async fn cmd_echo(
    selector: Option<&str>,
    origin: Option<&str>,
    class: Option<&str>,
    producer: Option<&str>,
    fmt: Option<&str>,
    raw: bool,
    hex_payload: bool,
    rate: bool,
    no_decode: bool,
    count: usize,
    args: &BusArgs,
) -> Result<()> {
    let selector = match selector {
        Some(s) => s.to_string(),
        None => compose_selector(args, origin, class, producer)?,
    };
    let base = args.base().to_string();

    // Slices first (a single introspect fan-in), then subscribe: the slice
    // set names each subject's payload type; the schema store fetches
    // `describe` lazily on first decode miss.
    let slices = if raw {
        zenkey_fleet::SliceSet::default()
    } else {
        args.slice_set().await?
    };
    let store = zenkey_fleet::decode::SchemaStore::new(&base, args.timeout());

    let session = args.session().await?;
    // Through the Monitor (issue #48): the same bounded broadcast the GUI
    // uses, so a bus that outruns this terminal surfaces as an explicit
    // dropped count instead of invisible loss (RFC 09 §5.1 O6). This is also
    // the §6.3 promise kept: the CLI validates the engine's path.
    let monitor = zenkey_fleet::Monitor::start(
        &session,
        zenkey_fleet::MonitorSpec {
            selectors: vec![selector.clone()],
            ..Default::default()
        },
    )
    .await?;
    let mut events = monitor.events();

    let ndjson = matches!(args.format.resolved(), output::Format::Ndjson);
    if !ndjson {
        eprintln!("echoing {selector} (ctrl-c to stop)");
    }
    let mut seen = 0usize;
    let mut dropped_total = 0u64;
    while let Some(item) = events.recv().await {
        let sample = match item {
            zenkey_fleet::StreamItem::Dropped(n) => {
                dropped_total += n;
                if ndjson {
                    println!("{}", serde_json::json!({ "dropped": n }));
                } else {
                    eprintln!("-- dropped {n} sample(s): the bus outran us --");
                }
                continue;
            }
            zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::Sample(s)) => s,
            zenkey_fleet::StreamItem::Event(_) => continue,
        };
        seen += 1;
        let key = sample.key.clone();
        let bytes = sample.payload.to_bytes();
        let encoding = sample.encoding.clone();
        let timestamp = sample.timestamp.map(|t| t.to_string());
        let rate_suffix = if rate {
            let (_, _, hz) = monitor.core().with_stats(|s| s.totals());
            format!("  @ {hz:.1}/s")
        } else {
            String::new()
        };

        if raw {
            println!("{key}\n  {}{rate_suffix}", hex(&bytes));
        } else if hex_payload {
            // --hex: the decode pipeline still names the type, the payload
            // shows as bytes.
            let (type_name, _) = zenkey_fleet::decode::decode_sample(
                &store,
                &session,
                &slices,
                &base,
                &key,
                Some(&encoding),
                &bytes,
            )
            .await;
            let tag = type_name
                .map(|t| format!("<{t}>"))
                .unwrap_or_else(|| "<unregistered>".to_string());
            println!("{key}\n  {tag} {}{rate_suffix}", hex(&bytes));
        } else {
            let (type_name, rendering) = if no_decode {
                (
                    None,
                    zenkey_fleet::decode::Rendering::Structural(zenkey_fleet::decode::structural(
                        &bytes,
                    )),
                )
            } else {
                zenkey_fleet::decode::decode_sample(
                    &store,
                    &session,
                    &slices,
                    &base,
                    &key,
                    Some(&encoding),
                    &bytes,
                )
                .await
            };
            let (value, typed, notes) = match &rendering {
                zenkey_fleet::decode::Rendering::Typed(d) => (
                    serde_json::to_string(&d.value).unwrap_or_default(),
                    true,
                    d.notes.clone(),
                ),
                zenkey_fleet::decode::Rendering::Structural(text) => {
                    (text.clone(), false, Vec::new())
                }
            };
            if ndjson {
                let parsed = zenkey::grammar::parse_full(&base, &key);
                let obj = serde_json::json!({
                    "key": key,
                    "origin": parsed.as_ref().map(|p| p.origin.chunk().to_string()),
                    "subject": parsed.as_ref().map(|p| p.subject.join("/")),
                    "type": type_name,
                    "typed": typed,
                    "encoding": encoding,
                    "timestamp": timestamp,
                    "value": serde_json::from_str::<serde_json::Value>(&value)
                        .unwrap_or(serde_json::Value::String(value.clone())),
                });
                println!("{obj}");
            } else if let Some(fmt) = fmt {
                println!(
                    "{}",
                    format_sample(
                        fmt,
                        seen,
                        &key,
                        &base,
                        type_name.as_deref(),
                        &encoding,
                        bytes.len(),
                        timestamp.as_deref(),
                        &value,
                    )
                );
            } else {
                let tag = match (&type_name, typed) {
                    (Some(t), true) => format!("<{t}>"),
                    (Some(t), false) => format!("<{t}?>"),
                    (None, _) => "<unregistered>".to_string(),
                };
                println!("{key}\n  {tag} {value}{rate_suffix}");
                for note in notes {
                    eprintln!("  note: {note}");
                }
            }
        }

        if count > 0 && seen >= count {
            break;
        }
    }
    if !ndjson {
        eprintln!(
            "{seen} sample(s) shown, {dropped_total} dropped{}",
            if dropped_total > 0 {
                " (the terminal could not keep up; the counts are the honest record)"
            } else {
                ""
            }
        );
    }
    Ok(())
}

/// `topic hz` / `topic bw` — watch a window, report rates (ros2-style).
#[allow(clippy::too_many_arguments)]
async fn cmd_rate(
    selector: Option<&str>,
    origin: Option<&str>,
    class: Option<&str>,
    producer: Option<&str>,
    window: u64,
    per_key: bool,
    loss: bool,
    bandwidth: bool,
    args: &BusArgs,
) -> Result<()> {
    let selector = match selector {
        Some(s) => s.to_string(),
        None => compose_selector(args, origin, class, producer)?,
    };
    let session = args.session().await?;
    let monitor = zenkey_fleet::Monitor::start(
        &session,
        zenkey_fleet::MonitorSpec {
            selectors: vec![selector.clone()],
            ..Default::default()
        },
    )
    .await?;

    eprintln!("measuring {selector} for {window}s…");
    tokio::time::sleep(Duration::from_secs(window)).await;

    let secs = window as f64;
    monitor.core().with_stats(|stats| {
        let (count, bytes, _) = stats.totals();
        if per_key {
            let mut rows: Vec<(String, u64, u64, u64)> = stats
                .iter()
                .map(|(k, s)| (k.to_string(), s.count, s.bytes, s.sn_gaps))
                .collect();
            rows.sort_by_key(|r| std::cmp::Reverse(r.1));
            for (key, count, bytes, gaps) in rows {
                if bandwidth {
                    println!("{:>12.1} B/s  {key}", bytes as f64 / secs);
                } else {
                    print!("{:>8.2} Hz  {key}", count as f64 / secs);
                    if loss {
                        print!("  ({gaps} sn gap(s))");
                    }
                    println!();
                }
            }
        }
        if stats.evicted() > 0 {
            // The table is bounded; a shrunken key set must say so (O6).
            eprintln!(
                "note: {} key(s) retired to stay within the {}-key bound — totals cover the retained set",
                stats.evicted(),
                stats.max_keys()
            );
        }
        if bandwidth {
            println!(
                "total: {:.1} B/s over {} key(s) ({bytes} bytes / {window}s)",
                bytes as f64 / secs,
                stats.len()
            );
        } else {
            print!(
                "total: {:.2} Hz over {} key(s) ({count} samples / {window}s)",
                count as f64 / secs,
                stats.len()
            );
            if loss {
                let gaps: u64 = stats.iter().map(|(_, s)| s.sn_gaps).sum();
                print!(
                    "  — {gaps} source-sn gap(s) (zero also means \"publishers attach no SourceInfo\")"
                );
            }
            println!();
        }
    });
    monitor.stop();
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `topic pub` — publish through the write facade (issue #47).
#[allow(clippy::too_many_arguments)]
async fn cmd_pub(
    key: &str,
    body: &str,
    qos: &str,
    encoding: Option<&str>,
    repeat: usize,
    interval: f64,
    no_validate: bool,
    args: &BusArgs,
) -> Result<()> {
    let qos = zenkey::qos::QosProfile::from_name(qos).ok_or_else(|| {
        anyhow!(
            "unknown QoS profile {qos:?} — sampled|refreshed|transition|alert|frame (RFC 04 §3)"
        )
    })?;
    let payload = match body {
        "-" => {
            use std::io::Read as _;
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf)?;
            buf
        }
        b => match b.strip_prefix('@') {
            Some(path) => std::fs::read(path)?,
            None => b.as_bytes().to_vec(),
        },
    };

    // Registry awareness: when the key refines to a registered subject the
    // declared encoding fills in, and a served schema validates the body by
    // ENCODING it — a body the schema cannot encode is refused before it
    // touches the bus (--no-validate opts out; an unregistered key publishes
    // as-is, honestly labelled).
    let mut declared_encoding = encoding.map(str::to_string);
    if !no_validate {
        let slices = args.slice_set().await.unwrap_or_default();
        let description = zenkey_fleet::describe_key(args.base(), key, Some(&slices));
        if let zenkey_fleet::Registration::Registered(subject) = &description.facts.registration {
            if declared_encoding.is_none() {
                declared_encoding = subject.encoding.clone();
            }
            let session = args.session().await?;
            let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
            let producer = subject_producer(&description);
            if let Some(producer) = producer
                && let Some(schema) = store
                    .schema_for(&session, &producer, &subject.type_name)
                    .await
            {
                let value: serde_json::Value =
                    serde_json::from_slice(&payload).map_err(|e| {
                        anyhow!(
                            "body is not JSON but {} declares schema-validated type {} — {e}                              (--no-validate to publish anyway)",
                            key,
                            subject.type_name
                        )
                    })?;
                let target = zenkey_fleet::decode::resolve_encoding(
                    declared_encoding.as_deref(),
                    subject.encoding.as_deref(),
                    &payload,
                );
                zenkey::schema::decode::DecoderRegistry::new()
                    .encode(&schema, &value, &target)
                    .map_err(|e| {
                        anyhow!(
                            "body rejected by {}'s served schema: {e} (--no-validate to                              publish anyway)",
                            subject.type_name
                        )
                    })?;
            }
        } else {
            eprintln!(
                "note: {key} is not a registered subject ({:?}) — publishing as-is",
                description.facts.registration
            );
        }
    }

    let session = args.session().await?;
    let publication =
        zenkey_fleet::declare_publication(&session, key, qos, declared_encoding.as_deref()).await?;
    let times = repeat.max(1);
    for n in 0..times {
        publication.send(payload.clone()).await?;
        eprintln!(
            "published {key} ({} bytes) [{}/{times}]",
            payload.len(),
            n + 1
        );
        if n + 1 < times {
            tokio::time::sleep(Duration::from_secs_f64(interval.max(0.0))).await;
        }
    }
    publication.undeclare().await?;
    Ok(())
}

/// The producer a registered description refined through (host producer or
/// the service slice's name is not carried on SubjectFacts — derive from the
/// shape).
fn subject_producer(d: &zenkey_fleet::KeyDescription) -> Option<String> {
    match &d.facts.shape {
        zenkey_fleet::KeyShape::V1(v) => v.producer.clone(),
        _ => None,
    }
}

/// `service call` — a GET on the `@rpc` plane (RFC 05).
async fn cmd_service_call(
    origin: &str,
    producer: &str,
    procedure: &str,
    params: &[String],
    body: Option<&str>,
    no_validate: bool,
    args: &BusArgs,
) -> Result<()> {
    // The typed target refuses a hostname outright (RFC 06 §6) and makes a
    // fleet call a deliberate variant; the engine's `call` composes the key
    // through the typed builders and applies the fan-in discipline plus the
    // registry-layer fanout guard (issue #36).
    let target = zenkey_fleet::CallTarget::parse(origin)?;

    let payload = match body {
        Some(b) => Some(match b.strip_prefix('@') {
            Some(path) => std::fs::read(path)?,
            None => b.as_bytes().to_vec(),
        }),
        None => None,
    };

    // The fanout guard needs slices; loading them costs one introspect
    // fan-in. --no-validate skips it (and with it the registry-layer refusal
    // — the generated-builder and ACL layers remain).
    let slices = if no_validate {
        None
    } else {
        args.slice_set().await.ok()
    };

    let session = args.session().await?;
    let report = zenkey_fleet::call(
        &session,
        args.base(),
        &target,
        producer,
        procedure,
        params,
        payload,
        args.timeout(),
        slices.as_ref(),
    )
    .await?;

    output::call(&report, args.format, |a| match (&a.value, &a.text) {
        (Some(v), _) => serde_json::to_string_pretty(v).unwrap_or_default(),
        (None, Some(t)) => t.clone(),
        _ => String::new(),
    });
    // Exit-code discipline preserved: 1 = an error reply, 2 = zero replies
    // (silence stays a distinct non-verdict — RFC 05 §3.1).
    let code = report.exit_code();
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

async fn cmd_doctor(deep: bool, args: &BusArgs) -> Result<()> {
    let session = args.session().await?;
    let roster = bus::roster(&session, args.base(), args.timeout()).await?;

    let mut findings = 0usize;
    let mut answered = 0usize;

    if args.registry.is_empty() {
        eprintln!(
            "note: no --registry <dir> given — skipping the served-vs-declared diff; only the \
             roster-vs-introspect check runs."
        );
    }
    let locals = offline::load_slices(&args.registry)?;

    for local in &locals {
        // A service origin's verbatim `@` chunk is structurally unmatchable by
        // the `*` of a fleet selector (property D4), so it takes its own key.
        // That is the grammar working, not an exception to it.
        let key = match &local.service_origin {
            Some(origin) => {
                let o = zenkey::ServiceOrigin::new(origin).map_err(|e| {
                    anyhow!("bad service origin in local slice {}: {e}", local.name)
                })?;
                args.wire(zenkey::selector::service_rpc(&o, &["introspect"]))?
            }
            None => args.wire(zenkey::selector::fleet_rpc(&local.name, &["introspect"]))?,
        };

        let answers = bus::fleet_get(&session, args.base(), &key, None, args.timeout()).await?;

        for answer in &answers {
            let bus::Answer::Value(bytes) = &answer.answer else {
                continue;
            };
            answered += 1;
            let served_toml = bytes.to_bytes();
            let served_toml = String::from_utf8_lossy(&served_toml);
            let served = match zenkey::parse_slice(&served_toml) {
                Ok(s) => s,
                Err(e) => {
                    println!(
                        "✗ {}/{}: served slice does not parse: {e}",
                        answer.origin, local.name
                    );
                    findings += 1;
                    continue;
                }
            };
            let diff = zenkey::slice::diff(&served, local);
            if diff.is_empty() {
                println!(
                    "✓ {}/{}: in sync (registry {})",
                    answer.origin, local.name, served.version
                );
            } else {
                for f in &diff {
                    println!("✗ {}/{}: {}", answer.origin, local.name, f.summary());
                    findings += 1;
                }
            }
        }
    }

    // With no local registry the only introspect coverage we can count is the
    // fleet-wide wildcard.
    if locals.is_empty() {
        let pairs = bus::fleet_registry(&session, args.base(), args.timeout()).await?;
        answered = pairs.len();
    }

    // The roster is what makes silence legible (RFC 05 §3.1): a producer that
    // holds an `alive` token but did not answer `introspect` is a bug, because
    // producers MUST declare their @rpc queryables *before* their token —
    // "alive ⇒ callable" (RFC 04 §5).
    let live: usize = roster.values().map(Vec::len).sum();
    println!("\n{answered} introspect repl(y|ies) from {live} live producer(s).");
    if answered < live {
        println!(
            "⚠ {} live producer(s) did not answer introspect — alive ⇒ callable (RFC 04 §5), so",
            live - answered
        );
        println!("  this is a finding, not a boot race.");
        findings += live - answered;
    }

    // --- Admin reachability (issue #14) -------------------------------
    let routers = zenkey_fleet::routers(&session, args.timeout())
        .await
        .unwrap_or_default();
    if routers.is_empty() {
        println!(
            "\nadmin: no routers answered @/*/router (peer-only mesh, or the admin \
             space is disabled) — storage/version checks skipped."
        );
    } else {
        let versions: std::collections::BTreeSet<&str> = routers
            .iter()
            .filter_map(|r| r.version.as_deref())
            .collect();
        if versions.len() > 1 {
            println!(
                "✗ admin: router version skew across the mesh: {:?}",
                versions
            );
            findings += 1;
        } else {
            println!(
                "\nadmin: {} router(s), version {}",
                routers.len(),
                versions.iter().next().copied().unwrap_or("-")
            );
        }
    }

    // --- Schema conformance (RFC 08 §7, issue #14) --------------------
    // Which slices to judge: the locals when given, else what the fleet
    // serves.
    let schema_slices: Vec<RegistrySlice> = if locals.is_empty() {
        bus::fleet_registry(&session, args.base(), args.timeout())
            .await?
            .into_iter()
            .map(|(_, s)| s)
            .collect()
    } else {
        locals.clone()
    };
    let mut described: Vec<(String, zenkey::schema::SchemaSet)> = Vec::new();
    let mut undescribed = 0usize;
    for slice in &schema_slices {
        let key = match &slice.service_origin {
            Some(origin) => {
                let o = zenkey::ServiceOrigin::new(origin)
                    .map_err(|e| anyhow!("bad service origin in slice {}: {e}", slice.name))?;
                args.wire(zenkey::selector::service_rpc(&o, &["describe"]))?
            }
            None => args.wire(zenkey::selector::fleet_rpc(&slice.name, &["describe"]))?,
        };
        let answers = bus::fleet_get(&session, args.base(), &key, None, args.timeout()).await?;
        let set = answers.into_iter().find_map(|a| match a.answer {
            bus::Answer::Value(bytes) => {
                let cow = bytes.to_bytes();
                std::str::from_utf8(&cow)
                    .ok()
                    .and_then(|t| zenkey::schema::SchemaSet::parse(t).ok())
            }
            bus::Answer::Error { .. } => None,
        });
        match set {
            Some(set) => {
                let names = referenced_type_names(slice);
                let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                if let Err(e) = set.verify_covers(&refs) {
                    println!("✗ {}: describe is not total: {e}", slice.name);
                    findings += 1;
                }
                described.push((slice.name.clone(), set));
            }
            None => undescribed += 1,
        }
    }
    for drift in schema_drift(&described) {
        println!("✗ {drift}");
        findings += 1;
    }
    if undescribed > 0 {
        println!(
            "schema: {undescribed} producer(s) serve no describe (a SHOULD — RFC 08 §7; \
             generic tools render their payloads structurally)."
        );
    }
    if !described.is_empty() {
        println!("schema: {} producer(s) serve describe.", described.len());
    }

    // --- --deep: freshness + storage coverage -------------------------
    if deep {
        let now = std::time::SystemTime::now();
        let mut unstamped = 0usize;
        for slice in &schema_slices {
            for subject in &slice.subjects {
                let (Some(ttl), "state") = (subject.ttl_s, subject.class.as_str()) else {
                    continue;
                };
                let Ok(pattern) = zenkey::pattern::SubjectPattern::parse(&subject.path) else {
                    continue;
                };
                let selector = match &slice.service_origin {
                    Some(origin) => {
                        args.wire(format!("v1/{origin}/state/{}", pattern.selector_tail()))?
                    }
                    None => args.wire(format!(
                        "v1/*/state/{}/{}",
                        slice.name,
                        pattern.selector_tail()
                    ))?,
                };
                for sample in bus::state_snapshot(&session, &selector, args.timeout()).await? {
                    match sample.timestamp {
                        Some(ts) => {
                            let stamped = ts.get_time().to_system_time();
                            if let Ok(age) = now.duration_since(stamped)
                                && age.as_secs() as i64 > ttl
                            {
                                println!(
                                    "✗ stale state: {} is {}s old against ttl {ttl}s \
                                     (RFC 04 §1.2: refresh <= ttl/2)",
                                    sample.key,
                                    age.as_secs()
                                );
                                findings += 1;
                            }
                        }
                        None => unstamped += 1,
                    }
                }
            }
        }
        if unstamped > 0 {
            println!(
                "⚠ {unstamped} state sample(s) carry no HLC timestamp — the deployment \
                 lacks timestamping, which RFC 04 §4 requires for LWW; freshness is \
                 unjudgeable for them."
            );
        }
        let storages = zenkey_fleet::storages(&session, args.timeout())
            .await
            .unwrap_or_default();
        let coverage = zenkey_fleet::state_coverage(
            &zenkey_fleet::SliceSet::from_slices(schema_slices.clone()),
            args.base(),
            &storages,
        );
        let uncovered: Vec<&zenkey_fleet::CoverageRow> = coverage
            .iter()
            .filter(|r| r.coverage == zenkey_fleet::Coverage::Uncovered)
            .collect();
        if !uncovered.is_empty() {
            println!(
                "storage: {} state famil(y|ies) have no storage coverage (informational — \
                 volatile seeding may ride the advanced-pub/sub cache, RFC 04 §3.5): {}",
                uncovered.len(),
                uncovered
                    .iter()
                    .map(|r| format!("{}/{}", r.producer, r.path))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }

    if findings == 0 {
        println!("\nno findings — the fleet agrees with this build.");
    } else {
        println!("\n{findings} finding(s).");
    }
    Ok(())
}

/// Every type name a slice references (subjects, procedure request/reply, and
/// since v1.8 blob `reference`s) — the RFC 08 §7 totality bound's right-hand
/// side. This list must track §7 exactly: a name §7 requires and this omits is
/// a coverage gap `doctor` would report as clean.
fn referenced_type_names(slice: &RegistrySlice) -> Vec<String> {
    let mut names: Vec<String> = slice
        .subjects
        .iter()
        .map(|s| s.type_name.clone())
        .filter(|t| !t.is_empty())
        .chain(slice.procedures.iter().filter_map(|p| p.request.clone()))
        .chain(slice.procedures.iter().filter_map(|p| p.reply.clone()))
        .chain(slice.blob.iter().filter_map(|b| b.reference.clone()))
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// Cross-producer schema drift (issue #14): the same type name served with
/// different hashes is exactly what the RFC 08 §7 hashes exist to catch.
fn schema_drift(described: &[(String, zenkey::schema::SchemaSet)]) -> Vec<String> {
    use std::collections::BTreeMap;
    let mut seen: BTreeMap<&str, (&str, &str)> = BTreeMap::new(); // type -> (producer, hash)
    let mut findings = Vec::new();
    for (producer, set) in described {
        for (name, schema) in set.iter() {
            match seen.get(name) {
                Some((other, hash)) if *hash != schema.hash() => findings.push(format!(
                    "schema drift: type {name:?} served by {other} ({hash}) and {producer} ({}) \
                     with different schemas",
                    schema.hash()
                )),
                Some(_) => {}
                None => {
                    seen.insert(name, (producer, schema.hash()));
                }
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_sample_extracts_payload_fields() {
        let line = format_sample(
            "%{iface.name} up=%{iface.up} missing=[%{no.such}]",
            1,
            "k",
            "",
            None,
            "application/json",
            2,
            None,
            r#"{"iface":{"name":"eth0","up":true}}"#,
        );
        assert_eq!(line, "eth0 up=true missing=[]");
    }

    #[test]
    fn format_sample_expands_fields() {
        let line = format_sample(
            "%n %o %c/%p %s <%t> %v (%l B, %e)",
            3,
            "tcgui/v1/h-3fa9c2d41b7e/state/tc/iface/eth0/state",
            "tcgui",
            Some("NetworkInterface"),
            "application/json",
            12,
            None,
            r#"{"up":true}"#,
        );
        assert_eq!(
            line,
            r#"3 h-3fa9c2d41b7e state/tc iface/eth0/state <NetworkInterface> {"up":true} (12 B, application/json)"#
        );
        // Escapes and literals.
        assert_eq!(
            format_sample(
                "%%|%K\\t.",
                1,
                "b/v1/h-3fa9c2d41b7e/state/tc/x",
                "b",
                None,
                "e",
                0,
                None,
                "v"
            ),
            "%|v1/h-3fa9c2d41b7e/state/tc/x\t."
        );
    }

    #[test]
    fn schema_drift_catches_same_name_different_hash() {
        use zenkey::schema::{SchemaSet, TypeSchema};
        let a = SchemaSet::builder("app")
            .entry(
                "Point",
                TypeSchema::json_schema(serde_json::json!({"a": 1})),
            )
            .build();
        let same = SchemaSet::builder("app")
            .entry(
                "Point",
                TypeSchema::json_schema(serde_json::json!({"a": 1})),
            )
            .build();
        let different = SchemaSet::builder("app")
            .entry(
                "Point",
                TypeSchema::json_schema(serde_json::json!({"a": 2})),
            )
            .build();
        assert!(schema_drift(&[("p1".into(), a.clone()), ("p2".into(), same)]).is_empty());
        let findings = schema_drift(&[("p1".into(), a), ("p3".into(), different)]);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].contains("Point"), "{findings:?}");
    }

    #[test]
    fn referenced_type_names_are_total_and_deduped() {
        let slice = zenkey::parse_slice(
            r#"
            [registry]
            version = "1.0"
            app = "t"
            convention = 1
            [producer]
            name = "tc"
            [[subject]]
            path = "health"
            class = "state"
            type = "Health"
            [[procedure]]
            path = "x/set"
            kind = "write"
            request = "Cmd"
            reply = "Health"
            "#,
        )
        .unwrap();
        assert_eq!(referenced_type_names(&slice), vec!["Cmd", "Health"]);
    }

    #[test]
    fn compose_selector_places_positions() {
        let args = BusArgs {
            base: Some("zs".into()),
            context: None,
            registry: vec![],
            connect: vec![],
            listen: vec![],
            scouting: false,
            timeout: None,
            format: output::Format::Table,
        };
        assert_eq!(
            compose_selector(&args, None, None, None).unwrap(),
            "zs/v1/*/**"
        );
        assert_eq!(
            compose_selector(&args, Some("h-3fa9c2d41b7e"), Some("state"), None).unwrap(),
            "zs/v1/h-3fa9c2d41b7e/state/**"
        );
        assert_eq!(
            compose_selector(&args, None, None, Some("tc")).unwrap(),
            "zs/v1/*/*/tc/**"
        );
        assert!(compose_selector(&args, None, Some("alerts"), None).is_err());

        // The empty base composes bare `v1/…` selectors (observer identity).
        let args = BusArgs {
            base: Some(String::new()),
            ..args
        };
        assert_eq!(
            compose_selector(&args, None, None, None).unwrap(),
            "v1/*/**"
        );
        assert_eq!(
            compose_selector(&args, None, Some("state"), None).unwrap(),
            "v1/*/state/**"
        );
    }
}

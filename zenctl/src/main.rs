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

mod bus;
mod context;
mod offline;
mod output;
mod report;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, anyhow};
use clap::{Args, Parser, Subcommand};
use zenkey::RegistrySlice;

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
    /// Procedures on the `@rpc` plane.
    #[command(subcommand)]
    Service(ServiceCmd),
    /// Payload types declared by the registry slices.
    #[command(subcommand)]
    Interface(InterfaceCmd),
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
    Doctor(BusArgs),
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
    Echo {
        /// Key expression to subscribe to. Defaults to all v1 data under the
        /// base: `<base>/v1/**`.
        selector: Option<String>,
        /// Print raw payload bytes as hex instead of decoding.
        #[arg(long)]
        raw: bool,
        /// Stop after this many samples (0 = run until interrupted).
        #[arg(long, default_value_t = 0)]
        count: usize,
        #[command(flatten)]
        bus: BusArgs,
    },
}

#[derive(Subcommand)]
enum NodeCmd {
    /// List live producers from the liveliness roster (on-bus).
    List(BusArgs),
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
    /// Optional since v1.5: resolution is flag > env > active context
    /// (`zenctl context …`).
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
    /// The deployment base: flag > env > active context.
    fn base(&self) -> Result<&str> {
        if let Some(b) = &self.base {
            return Ok(b.as_str());
        }
        self.stored()
            .and_then(|c| c.base.as_deref())
            .ok_or_else(|| {
                anyhow!(
                    "no base: pass --base, set ZENCTL_BASE, or create a context \
                     (`zenctl context create lab --base <base> -c <endpoint>`)"
                )
            })
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
        Ok(zenkey::grammar::with_base(self.base()?, relative))
    }
    /// Registry slices from whichever source the flags select: local
    /// `--registry` dirs when given (offline), otherwise the live bus
    /// (RFC 08 §6 introspection). Both yield the same `Vec<RegistrySlice>`,
    /// so every renderer is source-agnostic.
    async fn slices(&self) -> Result<Vec<RegistrySlice>> {
        let dirs = self.registry_dirs();
        if !dirs.is_empty() {
            return offline::load_slices(&dirs);
        }
        let session = self.session().await?;
        let base = self.base()?;
        let pairs = bus::fleet_registry(&session, base, self.timeout()).await?;
        if pairs.is_empty() {
            eprintln!(
                "no introspect slices on base {base:?} — an empty set is not a verdict (RFC 05 §3.1); \
                 `zenctl node list --base {base}` says who is actually up.\n\
                 (offline alternative: --registry <dir> with the app's registry TOMLs)"
            );
        }
        Ok(pairs.into_iter().map(|(_, slice)| slice).collect())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
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
            let report = offline::topic_info(bus.base()?, &key, &slices)?;
            output::topic_info(&report, bus.format)
        }
        Command::Topic(TopicCmd::Echo {
            selector,
            raw,
            count,
            bus,
        }) => cmd_echo(selector.as_deref(), raw, count, &bus).await,
        Command::Node(NodeCmd::List(bus)) => cmd_node_list(&bus).await,
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
            bus,
        }) => {
            cmd_service_call(
                &origin,
                &producer,
                &procedure,
                &params,
                body.as_deref(),
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
        Command::Doctor(bus) => cmd_doctor(&bus).await,
    }
}

/// `node list` — the liveliness roster (RFC 04 §5).
async fn cmd_node_list(args: &BusArgs) -> Result<()> {
    let session = args.session().await?;
    let roster = bus::roster(&session, args.base()?, args.timeout()).await?;

    if roster.is_empty() {
        eprintln!(
            "nothing held a liveliness token on {} — check --connect (and that \
             producers are actually running).",
            zenkey::selector::all_liveliness(zenkey::selector::Scope::fleet())
        );
    }
    output::node_list(&report::NodeList { origins: roster }, args.format)
}

/// `topic echo` — subscribe, refine each key against the registry slices,
/// render the payload generically.
///
/// Subscribe-first is not a style choice: RFC 04 §3.2 forbids GET-then-subscribe
/// (it drops everything published in the gap). This only subscribes, so it is
/// trivially correct — but a `--seed` flag would have to keep that order.
async fn cmd_echo(selector: Option<&str>, raw: bool, count: usize, args: &BusArgs) -> Result<()> {
    let selector = selector
        .map(str::to_string)
        .map(Ok)
        .unwrap_or_else(|| args.wire("v1/**"))?;

    // Slices first (a single introspect fan-in), then subscribe: the slice
    // set names each subject's payload type, which is what makes the rendered
    // lines legible without anything compiled in.
    let slices = if raw {
        Vec::new()
    } else {
        args.slices().await?
    };

    let session = args.session().await?;
    let subscriber = session
        .declare_subscriber(&selector)
        .await
        .map_err(|e| anyhow!("{e}"))?;

    eprintln!("echoing {selector} (ctrl-c to stop)");
    let mut seen = 0usize;
    while let Ok(sample) = subscriber.recv_async().await {
        let key = sample.key_expr().as_str().to_string();
        let bytes = sample.payload().to_bytes();

        if raw {
            println!("{key}\n  {}", hex(&bytes));
        } else {
            println!("{key}\n  {}", render(args.base()?, &key, &bytes, &slices));
        }

        seen += 1;
        if count > 0 && seen >= count {
            break;
        }
    }
    Ok(())
}

/// Wire key → subject (via the slices) → generically rendered value, with
/// nothing app-specific compiled in.
fn render(base: &str, key: &str, bytes: &[u8], slices: &[RegistrySlice]) -> String {
    let type_name = zenkey::grammar::parse_full(base, key)
        .and_then(|parsed| {
            let producer = parsed.producer.as_ref().map(|p| p.name().to_string())?;
            let slice = slices.iter().find(|s| s.name == producer)?;
            let class = match parsed.class {
                zenkey::grammar::ClassOrPlane::Class(c) => c.chunk(),
                zenkey::grammar::ClassOrPlane::Plane(p) => p.chunk(),
            };
            let tail: &[&str] = &parsed.subject;
            slice
                .subjects
                .iter()
                .find(|s| s.class == class && offline::match_subject(&s.path, tail).is_some())
                .map(|s| s.type_name.clone())
        })
        .unwrap_or_else(|| "unregistered".to_string());
    format!("<{type_name}> {}", render_value(bytes))
}

/// Best-effort generic payload rendering. The convention's profile default is
/// CBOR with a first-byte sniff for JSON interop, so: payloads that look like
/// JSON text parse as JSON; otherwise CBOR → JSON diagnostic; otherwise UTF-8
/// text; otherwise hex. A generic explorer cannot synthesize a foreign app's
/// Rust types (RFC 08 §5 keeps schema definitions in the owning crates) —
/// this shows the *structure*, which is what the wire honestly says.
fn render_value(bytes: &[u8]) -> String {
    let looks_json = bytes.first().is_some_and(|b| {
        matches!(
            b,
            b'{' | b'[' | b'"' | b' ' | b'-' | b'0'..=b'9' | b't' | b'f' | b'n'
        )
    });
    if looks_json && let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) {
        return serde_json::to_string(&v).unwrap_or_default();
    }
    if let Ok(v) = ciborium::from_reader::<ciborium::Value, _>(bytes)
        && let Ok(text) = serde_json::to_string(&v)
    {
        return text;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) if !text.is_empty() => text.to_string(),
        _ => format!("<{} bytes> {}", bytes.len(), hex(bytes)),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `service call` — a GET on the `@rpc` plane (RFC 05).
async fn cmd_service_call(
    origin: &str,
    producer: &str,
    procedure: &str,
    params: &[String],
    body: Option<&str>,
    args: &BusArgs,
) -> Result<()> {
    // A service origin (`@catalog`) carries no producer chunk (RFC 03 §1.5).
    // Un-namespaced tool ⇒ full keys, composed off the configured base.
    let mut key = if origin.starts_with('@') {
        args.wire(format!("v1/{origin}/@rpc/{procedure}"))?
    } else {
        args.wire(format!("v1/{origin}/@rpc/{producer}/{procedure}"))?
    };
    if !params.is_empty() {
        key.push('?');
        key.push_str(&params.join(";"));
    }

    let payload = match body {
        Some(b) => Some(match b.strip_prefix('@') {
            Some(path) => std::fs::read(path)?,
            None => b.as_bytes().to_vec(),
        }),
        None => None,
    };

    let session = args.session().await?;
    let answers = bus::fleet_get(&session, args.base()?, &key, payload, args.timeout()).await?;

    // Assemble the typed report (issue #12): value replies parse as JSON when
    // they are JSON (read replies, by convention) and ride as text otherwise
    // (introspect replies raw TOML). An error reply is a failure, never
    // dressed up as success (RFC 05 §3).
    let report = report::CallReport {
        key: key.clone(),
        answers: answers
            .iter()
            .map(|a| match &a.answer {
                bus::Answer::Value(bytes) => {
                    match serde_json::from_slice::<serde_json::Value>(bytes) {
                        Ok(v) => report::CallAnswer {
                            origin: a.origin.clone(),
                            ok: true,
                            value: Some(v),
                            text: None,
                            error: None,
                        },
                        Err(_) => report::CallAnswer {
                            origin: a.origin.clone(),
                            ok: true,
                            value: None,
                            text: Some(String::from_utf8_lossy(bytes).to_string()),
                            error: None,
                        },
                    }
                }
                bus::Answer::Error { name, message } => report::CallAnswer {
                    origin: a.origin.clone(),
                    ok: false,
                    value: None,
                    text: None,
                    error: Some(report::CallError {
                        name: name.clone(),
                        message: message.clone(),
                    }),
                },
            })
            .collect(),
    };
    output::call(&report, args.format, |a| match (&a.value, &a.text) {
        (Some(v), _) => serde_json::to_string_pretty(v).unwrap_or_default(),
        (None, Some(t)) => t.clone(),
        _ => String::new(),
    });
    // Exit-code discipline: 1 = an error reply, 2 = zero replies (silence
    // stays a distinct non-verdict).
    let code = report.exit_code();
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/// `doctor` — fan `introspect` across the fleet and diff each reply against
/// the local registry files (RFC 08 §6).
async fn cmd_doctor(args: &BusArgs) -> Result<()> {
    let session = args.session().await?;
    let roster = bus::roster(&session, args.base()?, args.timeout()).await?;

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

        let answers = bus::fleet_get(&session, args.base()?, &key, None, args.timeout()).await?;

        for answer in &answers {
            let bus::Answer::Value(bytes) = &answer.answer else {
                continue;
            };
            answered += 1;
            let served_toml = String::from_utf8_lossy(bytes);
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
        let pairs = bus::fleet_registry(&session, args.base()?, args.timeout()).await?;
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

    if findings == 0 {
        println!("no findings — the fleet agrees with this build.");
    } else {
        println!("{findings} finding(s).");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_value_sniffs_json_cbor_text_and_bytes() {
        // JSON text renders as JSON.
        assert_eq!(render_value(br#"{"a":1}"#), r#"{"a":1}"#);
        // CBOR renders as its JSON diagnostic.
        let mut cbor = Vec::new();
        ciborium::into_writer(&serde_json::json!({"b": 2}), &mut cbor).unwrap();
        assert_eq!(render_value(&cbor), r#"{"b":2}"#);
        // Plain text stays text; binary falls back to hex.
        assert_eq!(render_value(b"plain text"), "plain text");
        assert!(render_value(&[0xff, 0x00, 0x9c]).contains("ff 00 9c"));
    }

    #[test]
    fn render_tags_the_declared_type_and_unregistered() {
        let slice = zenkey::parse_slice(
            r#"
            [registry]
            version = "0.3"
            app = "tcgui"
            convention = 1
            [producer]
            name = "tc"
            [[subject]]
            path = "iface/{iface}/state"
            class = "state"
            type = "NetworkInterface"
            "#,
        )
        .unwrap();
        let slices = vec![slice];
        let line = render(
            "tcgui",
            "tcgui/v1/h-3fa9c2d41b7e/state/tc/iface/eth0/state",
            br#"{"up":true}"#,
            &slices,
        );
        assert_eq!(line, r#"<NetworkInterface> {"up":true}"#);

        let missing = render(
            "tcgui",
            "tcgui/v1/h-3fa9c2d41b7e/state/tc/bogus",
            b"x",
            &slices,
        );
        assert!(missing.starts_with("<unregistered>"), "got: {missing}");
    }
}

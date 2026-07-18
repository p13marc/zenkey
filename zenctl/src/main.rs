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
mod offline;

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
    /// Diff what the fleet *serves* against local registry files.
    ///
    /// RFC 08 §6: "A disagreement between introspection and the checked-in TOML
    /// is a finding, not an ambiguity." This prints the findings. The local
    /// truth comes from `--registry <dir>`; without it only the
    /// roster-vs-introspect check runs.
    Doctor(BusArgs),
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
    #[arg(long, env = "ZENCTL_BASE")]
    base: String,
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
    /// Seconds to wait for replies / to watch the bus.
    #[arg(long, default_value_t = 5)]
    timeout: u64,
}

impl BusArgs {
    async fn session(&self) -> Result<zenoh::Session> {
        bus::open(&self.connect, &self.listen, self.scouting).await
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout)
    }
    /// Compose a base-relative key into the full wire key this un-namespaced
    /// tool must actually use.
    fn wire(&self, relative: &str) -> String {
        zenkey::grammar::with_base(&self.base, relative)
    }
    /// Registry slices from whichever source the flags select: local
    /// `--registry` dirs when given (offline), otherwise the live bus
    /// (RFC 08 §6 introspection). Both yield the same `Vec<RegistrySlice>`,
    /// so every renderer is source-agnostic.
    async fn slices(&self) -> Result<Vec<RegistrySlice>> {
        if !self.registry.is_empty() {
            return offline::load_slices(&self.registry);
        }
        let session = self.session().await?;
        let pairs = bus::fleet_registry(&session, &self.base, self.timeout()).await?;
        if pairs.is_empty() {
            eprintln!(
                "no introspect slices on base {:?} — an empty set is not a verdict (RFC 05 §3.1); \
                 `zenctl node list --base {}` says who is actually up.\n\
                 (offline alternative: --registry <dir> with the app's registry TOMLs)",
                self.base, self.base
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
            offline::topic_list(&slices, producer.as_deref(), class.as_deref())
        }
        Command::Topic(TopicCmd::Info { key, bus }) => {
            let slices = bus.slices().await?;
            offline::topic_info(&bus.base, &key, &slices)
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
            offline::service_list(&slices, producer.as_deref())
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
            offline::interface_list(&slices)
        }
        Command::Interface(InterfaceCmd::Show { type_name, bus }) => {
            let slices = bus.slices().await?;
            offline::interface_show(&slices, &type_name)
        }
        Command::Doctor(bus) => cmd_doctor(&bus).await,
    }
}

/// `node list` — the liveliness roster (RFC 04 §5).
async fn cmd_node_list(args: &BusArgs) -> Result<()> {
    let session = args.session().await?;
    let roster = bus::roster(&session, &args.base, args.timeout()).await?;

    if roster.is_empty() {
        println!("no live producers.");
        println!(
            "\nnothing held a liveliness token on {}.",
            zenkey::grammar::all_liveliness_wildcard()
        );
        println!("if you expected some, check --connect (and that they are actually running).");
        return Ok(());
    }

    for (origin, producers) in &roster {
        println!("{origin}");
        for p in producers {
            println!("  {p}");
        }
    }
    let total: usize = roster.values().map(Vec::len).sum();
    println!("\n{total} producer(s) on {} origin(s).", roster.len());
    Ok(())
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
        .unwrap_or_else(|| args.wire("v1/**"));

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
            println!("{key}\n  {}", render(&args.base, &key, &bytes, &slices));
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
            let tail: Vec<&str> = parsed.subject.iter().map(String::as_str).collect();
            slice
                .subjects
                .iter()
                .find(|s| s.class == class && offline::match_subject(&s.path, &tail).is_some())
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
        args.wire(&format!("v1/{origin}/@rpc/{procedure}"))
    } else {
        args.wire(&format!("v1/{origin}/@rpc/{producer}/{procedure}"))
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
    eprintln!("GET {key}");
    let answers = bus::fleet_get(&session, &args.base, &key, payload, args.timeout()).await?;

    if answers.is_empty() {
        // RFC 05 §3.1: "no reply" is not one condition, and callers "MUST NOT
        // treat 'empty reply set' as a verdict about any specific host".
        println!("no replies.");
        println!("\nthat is not a verdict: an empty set means no queryable matched — an offline");
        println!("host, a mistyped origin, or a procedure this producer does not serve.");
        println!("`zenctl node list` says who is actually up.");
        return Ok(());
    }

    for a in &answers {
        match &a.answer {
            bus::Answer::Value(bytes) => {
                // Read replies are JSON by convention; introspect replies raw
                // TOML. Print what parses, fall back to the text.
                let text = match serde_json::from_slice::<serde_json::Value>(bytes) {
                    Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_default(),
                    Err(_) => String::from_utf8_lossy(bytes).to_string(),
                };
                println!("── {} ──\n{text}", a.origin);
            }
            bus::Answer::Error { name, message } => {
                // RFC 05 §3: an error reply always means failure. Never dress
                // one up as success.
                eprintln!("── {} ── {name}: {message}", a.origin);
            }
        }
    }
    Ok(())
}

/// `doctor` — fan `introspect` across the fleet and diff each reply against
/// the local registry files (RFC 08 §6).
async fn cmd_doctor(args: &BusArgs) -> Result<()> {
    let session = args.session().await?;
    let roster = bus::roster(&session, &args.base, args.timeout()).await?;

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
                let o = zenkey::grammar::Origin::service(origin).map_err(|e| {
                    anyhow!("bad service origin in local slice {}: {e}", local.name)
                })?;
                args.wire(&zenkey::grammar::service_rpc_key(&o, "introspect")?)
            }
            None => args.wire(&zenkey::grammar::fleet_rpc_key(&local.name, "introspect")),
        };

        let answers = bus::fleet_get(&session, &args.base, &key, None, args.timeout()).await?;

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
        let pairs = bus::fleet_registry(&session, &args.base, args.timeout()).await?;
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

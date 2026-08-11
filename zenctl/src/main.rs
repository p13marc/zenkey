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

mod cmd;
mod completion;
mod context;
mod offline;
mod output;
mod report;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use clap_complete::ArgValueCandidates;
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
    /// GET any selector with the fleet discipline (RFC 05 §2.1).
    ///
    /// Target All, consolidation None, every reply attributed by its own
    /// key; error envelopes render as errors (RFC 05 §3), and payloads ride
    /// the same rendering ladder as `topic echo` (served-schema decode →
    /// structural → text → hex). `@/**` browses the zenoh admin space.
    /// Exit codes: 0 values only, 1 an error reply, 2 silence.
    Get {
        /// Any key expression, params included (`key?k=v`).
        #[arg(add = ArgValueCandidates::new(completion::keys))]
        selector: String,
        /// Query body: inline text, `@file`, or `-` for stdin — rides the
        /// same encode ladder as `topic pub` when the selector's key part
        /// refines to a registered subject.
        #[arg(long)]
        body: Option<String>,
        /// Ship the body verbatim and print payloads as hex; no decode.
        #[arg(long)]
        raw: bool,
        /// Decode the type name, print the payload as hex.
        #[arg(long)]
        hex: bool,
        /// Per-reply line template — the `topic echo` % vocabulary
        /// (%k %K %o %c %p %s %t %v %e %l %n %{a.b.c}).
        #[arg(long, value_name = "TEMPLATE")]
        fmt: Option<String>,
        /// Skip schema decode; render structurally.
        #[arg(long)]
        no_decode: bool,
        #[command(flatten)]
        bus: BusArgs,
    },
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
    /// Dump a producer's served payload schemas (`@rpc/<producer>/describe`).
    ///
    /// RFC 08 §7: the shapes are served data. A producer that serves no
    /// `describe` degrades honestly — it is not an error.
    Schema {
        /// Producer name, e.g. `sysinfo`.
        #[arg(add = ArgValueCandidates::new(completion::producers))]
        producer: String,
        /// Show only this type (implies the full document).
        #[arg(long = "type", value_name = "TYPE", add = ArgValueCandidates::new(completion::types))]
        type_name: Option<String>,
        /// Print every schema document in full, not just kind + hash.
        #[arg(long)]
        full: bool,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// The registry as a document: export it, diff it, lint it.
    #[command(subcommand)]
    Registry(RegistryCmd),
    /// Measure the fleet.
    #[command(subcommand)]
    Bench(BenchCmd),
    /// Zenoh admin space (`@/**`) — the middleware's own introspection.
    #[command(subcommand)]
    Admin(AdminCmd),
    /// Storages: what the mesh persists, joined against declared state.
    #[command(subcommand)]
    Storage(StorageCmd),
    /// The `@blob` plane: who serves bulk content, and fetching it (RFC 07 §2).
    #[command(subcommand)]
    Blob(BlobCmd),
    /// Listen for raw scouting Hellos: zid, whatami, locators.
    ///
    /// The layer *below* `base list`: no session is opened, so this answers
    /// "is anything out there at all" and "is multicast working on this
    /// segment". It is also the one zenctl verb where multicast is ON by
    /// default — scouting is the point, and a scout only listens for Hellos
    /// and joins nothing.
    Scout {
        /// Filter by advertised kind. Repeatable; default: all three.
        #[arg(long, value_enum)]
        what: Vec<cmd::scout::ScoutWhat>,
        /// Seconds to listen (default 5; a context may override).
        #[arg(long)]
        timeout: Option<u64>,
        /// Endpoint to connect to, repeatable — reaches gossip scouting
        /// where multicast is filtered.
        #[arg(long, short = 'c')]
        connect: Vec<String>,
        /// Endpoint to listen on, repeatable.
        #[arg(long, short = 'l')]
        listen: Vec<String>,
        /// Use a named context for endpoints/timeout defaults.
        #[arg(long, value_name = "NAME", add = ArgValueCandidates::new(completion::contexts))]
        context: Option<String>,
        /// table = census (deduped by zid); ndjson = arrival log, one Hello
        /// per line as heard.
        #[arg(long, env = "ZENCTL_FORMAT", value_enum, default_value_t = output::Format::Auto)]
        format: output::Format,
    },
    /// Manage named connection contexts (config file).
    #[command(subcommand)]
    Context(ContextCmd),
    /// Inspect or clear the slice cache that feeds shell completion.
    #[command(subcommand)]
    Cache(CacheCmd),
    /// Generate shell completions (bash, zsh, fish, elvish, powershell).
    ///
    /// e.g. `zenctl completions bash > ~/.local/share/bash-completion/completions/zenctl`
    ///
    /// The default script is **dynamic** (issue #54): it calls back into
    /// `zenctl` so producer, subject, type and procedure names come from the
    /// cached registry of the active context. The cache is written by any
    /// command that loads slices; completion never touches the bus, so a
    /// `<TAB>` cannot hang on a fleet that is down. `--static` emits the old
    /// self-contained script instead.
    Completions {
        shell: clap_complete::Shell,
        /// Emit the self-contained script: no callbacks, no cached names.
        #[arg(long = "static")]
        static_only: bool,
    },
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
        /// With --deep: drain at most N state samples per family — bounds
        /// the sweep's cost, not just its output.
        #[arg(long, value_name = "N", requires = "deep")]
        sample: Option<usize>,
        /// Exit 1 when a finding at (or above) this severity exists.
        /// Default: always exit 0 — findings are output, not verdicts.
        #[arg(long, value_enum, value_name = "SEVERITY")]
        fail_on: Option<FailOn>,
        #[command(flatten)]
        bus: BusArgs,
    },
}

/// The `--fail-on` severity ceiling (scripting hook; opt-in).
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum FailOn {
    /// Fail on error-severity findings only.
    Error,
    /// Fail on warnings or errors.
    Warning,
}

#[derive(Subcommand)]
enum CacheCmd {
    /// Print the cache directory for the active context, and what is in it.
    Show {
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Re-read the slice source and rewrite the cache.
    ///
    /// Every command already answers from its source live — this exists so a
    /// *completion* can be brought up to date without running one, and so
    /// "why is it suggesting a producer we deleted" has an answer.
    Refresh {
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Delete the cache for the active context.
    Clear {
        #[command(flatten)]
        bus: BusArgs,
    },
}

#[derive(Subcommand)]
enum BenchCmd {
    /// Time an `@rpc` procedure, per origin.
    ///
    /// Latency is measured **per reply**, so a fast origin in a fan-out is not
    /// charged the slowest one's round trip. Only procedures the registry
    /// declares `idempotent = true` bench by default: a benchmark repeats, and
    /// repeating a write into a live fleet is a different act from measuring
    /// it.
    Rpc {
        /// Origin to target: a host id, `*` for the fleet, or `@catalog`.
        origin: String,
        /// Producer name.
        #[arg(add = ArgValueCandidates::new(completion::producers))]
        producer: String,
        /// Procedure path. `introspect` is the safe default: RFC 08 §6 makes
        /// it a read every producer serves.
        #[arg(default_value = "introspect", add = ArgValueCandidates::new(completion::procedures))]
        procedure: String,
        /// Calls to issue (default 100).
        #[arg(long)]
        count: Option<usize>,
        /// Calls in flight at once (1 = strictly sequential).
        #[arg(long, default_value_t = 1)]
        concurrency: usize,
        /// Bench a procedure the registry does not declare idempotent.
        #[arg(long = "i-know")]
        i_know: bool,
        #[command(flatten)]
        bus: BusArgs,
    },
}

#[derive(Subcommand)]
enum RegistryCmd {
    /// Export the loaded slice set as a document.
    ///
    /// `--as toml` round-trips through `--registry <dir>`; `--as jsonschema`
    /// bundles the producers' served `describe` schemas (RFC 08 §7);
    /// `--as asyncapi` maps subjects to channels and procedures to operations.
    Export {
        /// Output document (note: `--format` stays the table/json switch).
        #[arg(long = "as", value_enum, default_value = "toml")]
        target: cmd::registry::ExportAs,
        /// Only this producer.
        #[arg(long, add = ArgValueCandidates::new(completion::producers))]
        producer: Option<String>,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Diff local `--registry` files against what the fleet serves.
    ///
    /// RFC 08 §6: a disagreement is a finding, not an ambiguity. `doctor`
    /// judges a deployment; this just shows the two registries side by side.
    Diff {
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Run the RFC 08 §5 registry lints on a directory, as a build would.
    Lint {
        /// The registry directory (the one a build script points at).
        dir: PathBuf,
        /// Deprecation ledger; defaults to `<dir>/deprecated.lock`.
        #[arg(long, value_name = "FILE")]
        ledger: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum AdminCmd {
    // `admin get` was folded into the first-class `zenctl get` (#114): the
    // generic browse lost its misleading name, and the new verb keeps the
    // admin space reachable (`zenctl get '@/**'`) with strictly more honest
    // rendering (error envelopes, typed payloads, non-lossy bytes).
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
        /// Re-render on change every SECS seconds (default 2).
        #[arg(long, value_name = "SECS", num_args = 0..=1, default_missing_value = "2")]
        watch: Option<f64>,
        #[command(flatten)]
        bus: BusArgs,
    },
}

#[derive(Subcommand)]
enum BlobCmd {
    /// Which producers declare which `@blob` tiers (registry only, no bus
    /// traffic). A declaration is a capability, never possession.
    List {
        /// Only this producer's declarations.
        #[arg(long, add = ArgValueCandidates::new(completion::producers))]
        producer: Option<String>,
        /// Only this tier: artifact, tree or store.
        #[arg(long, add = ArgValueCandidates::new(completion::blob_tiers))]
        tier: Option<String>,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Ask every origin who holds an artifact, with a *tiny* reply
    /// (RFC 07 §2.5): `have` and `manifest`, at data-low, never the bytes.
    ///
    /// There is no `--origin`: fanning out is what a probe *is*. The
    /// content-addressed tiers have no probe endpoint — their key carries the
    /// object — and say so rather than reporting zero holders.
    Probe {
        /// `<id>`, `artifact/<id>`, `tree/<hex>` or `store/<algo>/<hex>`.
        target: String,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Fetch from **one** origin's concrete key, at data-low, verifying every
    /// reply against the content root before it reaches disk (RFC 07 §2.1).
    Fetch {
        /// `<id>`, `artifact/<id>`, `tree/<hex>` or `store/<algo>/<hex>`.
        target: String,
        /// The one origin to fetch from (`h-<12hex>` or `@service`) — as
        /// reported by `zenctl blob probe`. A wildcard is not an origin.
        #[arg(long = "from", value_name = "ORIGIN")]
        from: String,
        /// Where to write. Defaults to the target's last chunk; the origin's
        /// advisory filename is never used to choose a path.
        #[arg(long, short = 'o', value_name = "PATH")]
        out: Option<PathBuf>,
        /// The content root the reference carried (RFC 07 §2.1). Every reply
        /// is verified against it before disk.
        #[arg(long, value_name = "HEX")]
        root: Option<String>,
        /// Accept whatever this origin serves, without a root to check it
        /// against — trust-on-first-use, stated out loud.
        #[arg(long, conflicts_with = "root")]
        allow_unpinned: bool,
        /// Replace an existing destination file.
        #[arg(long)]
        overwrite: bool,
        /// Suppress progress on stderr.
        #[arg(long, short = 'q')]
        quiet: bool,
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
    /// Open the whole config file in $VISUAL/$EDITOR, validating afterwards.
    Edit,
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
        #[arg(long, add = ArgValueCandidates::new(completion::producers))]
        producer: Option<String>,
        /// Only this class: telemetry, state, or events.
        #[arg(long, add = ArgValueCandidates::new(completion::classes))]
        class: Option<String>,
        /// Only subjects carrying this payload type.
        #[arg(long, value_name = "TYPE", add = ArgValueCandidates::new(completion::types))]
        r#type: Option<String>,
        /// Also list retired subjects from each slice's `[[deprecated]]`
        /// ledger (RFC 08 §6: which hosts still serve a deprecated subject).
        #[arg(long)]
        deprecated: bool,
        /// Re-render on change every SECS seconds (default 2). Appeared and
        /// disappeared subjects are marked for one cycle; ndjson streams one
        /// snapshot object per cycle.
        #[arg(long, value_name = "SECS", num_args = 0..=1, default_missing_value = "2")]
        watch: Option<f64>,
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
        #[arg(add = ArgValueCandidates::new(completion::keys))]
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
        /// Seed current state before going live (RFC 04 §3.2, issue #92):
        /// subscribe first, then pull publishers' caches and router storages
        /// through one LWW merge; a boundary line marks where the seed ends
        /// and live begins.
        #[arg(long)]
        seed: bool,
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
        #[arg(long, default_value = "sampled", add = ArgValueCandidates::new(completion::qos_profiles))]
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
        /// Do not refuse a body the served schema rejects — it ships as typed,
        /// with a note. (It is still encoded when it *does* encode.)
        #[arg(long)]
        no_validate: bool,
        /// Send the bytes verbatim: no schema lookup, no encoding, no refusal.
        /// The escape hatch for a subject this tool cannot type.
        #[arg(long)]
        raw: bool,
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
    /// One node's full story: producers, versions, capabilities, freshness
    /// (issue #49; RFC 08 §6's capability-and-version inventory, per node).
    Info {
        /// The origin id (`h-<12hex>`). A hostname is refused — resolve it
        /// through the catalog first (RFC 06 §6).
        origin: String,
        #[command(flatten)]
        bus: BusArgs,
    },
    List {
        /// Join each producer against its served introspect slice (app +
        /// registry version).
        #[arg(long)]
        verbose: bool,
        /// Re-render on liveliness events (no polling — the bus pushes the
        /// roster). Reflects a producer stopping within one event.
        #[arg(long)]
        watch: bool,
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
        /// Re-render on change every SECS seconds (default 2).
        #[arg(long, value_name = "SECS", num_args = 0..=1, default_missing_value = "2")]
        watch: Option<f64>,
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
        #[arg(add = ArgValueCandidates::new(completion::producers))]
        producer: String,
        /// Procedure path, e.g. `introspect` or `artifact/status`.
        #[arg(add = ArgValueCandidates::new(completion::procedures))]
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
        /// Send the request body verbatim: no schema lookup, no encoding.
        #[arg(long)]
        raw: bool,
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
        #[arg(add = ArgValueCandidates::new(completion::types))]
        type_name: String,
        /// Also fetch the served schema from every producer that carries it
        /// (RFC 08 §7). Disagreeing hashes are reported as drift.
        #[arg(long)]
        schema: bool,
        /// With --schema, print each schema document in full.
        #[arg(long)]
        full: bool,
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
    #[arg(long, value_name = "NAME", add = ArgValueCandidates::new(completion::contexts))]
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
    /// The `--context` name this invocation was given, if any.
    fn context_name(&self) -> Option<&str> {
        self.context.as_deref()
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
                    "registry disagreement: {} — bus serves v{}, dirs carry v{}{}                      (served wins; `zenctl doctor --registry <dir>` details the drift)",
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
            self.cache(&out.set);
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
        self.cache(&set);
        Ok(set)
    }

    /// Persist the slice set for shell completion (issue #54).
    ///
    /// Best-effort by design: a cache that cannot be written must not fail the
    /// command the user actually ran, and an unwritable cache dir is a
    /// completion that stays static — not an outage. An *empty* set is never
    /// written, because overwriting a good cache with a sweep that found
    /// nothing would turn one bad moment on the bus into a permanently blank
    /// completion.
    fn cache(&self, set: &zenkey_fleet::SliceSet) {
        if set.slices().is_empty() {
            return;
        }
        let dir =
            zenkey_fleet::cache_dir(zenkey_fleet::active_name(self.context.as_deref()).as_deref());
        // Nothing is logged on failure: this runs on every command, and a
        // warning about a cache the user did not ask for would be noise on
        // the output they did.
        let _ = set.write_cache(&dir);
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
            let slices = bus.slices().await?;
            let report = filter.apply(&slices)?;
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
            qos,
            encoding,
            repeat,
            interval,
            no_validate,
            raw,
            bus,
        }) => {
            cmd::publish::run(
                &key,
                &body,
                &qos,
                encoding.as_deref(),
                repeat,
                interval,
                no_validate,
                raw,
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
            cmd::rate::run(
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
            cmd::rate::run(
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
            output::base_list(&report::BaseList { bases }, bus.format)
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
            output::storage_list(&report::StorageList { storages, coverage }, bus.format)
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
            let dest = out.unwrap_or_else(|| cmd::blob::default_dest(&target));
            cmd::blob::fetch(
                &target,
                &from,
                &dest,
                root.as_deref(),
                allow_unpinned,
                overwrite,
                quiet,
                &bus,
            )
            .await
        }
        Command::Admin(AdminCmd::Routers { bus }) => cmd::admin::routers(&bus).await,
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
            raw,
            bus,
        }) => {
            cmd::call::run(
                &origin,
                &producer,
                &procedure,
                &params,
                body.as_deref(),
                no_validate,
                raw,
                &bus,
            )
            .await
        }
        Command::Interface(InterfaceCmd::List { bus }) => {
            let slices = bus.slices().await?;
            let report = offline::interface_list(&slices)?;
            output::interface_list(&report, bus.format)
        }
        Command::Interface(InterfaceCmd::Show {
            type_name,
            schema,
            full,
            bus,
        }) => {
            let slices = bus.slices().await?;
            let mut report = offline::interface_show(&slices, &type_name)?;
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
            output::interface_show(&report, bus.format)
        }
        Command::Schema {
            producer,
            type_name,
            full,
            bus,
        } => cmd::schema::dump(&producer, type_name.as_deref(), full, &bus).await,
        Command::Registry(RegistryCmd::Export {
            target,
            producer,
            bus,
        }) => cmd::registry::export(target, producer.as_deref(), &bus).await,
        Command::Registry(RegistryCmd::Diff { bus }) => cmd::registry::diff(&bus).await,
        Command::Registry(RegistryCmd::Lint { dir, ledger }) => {
            cmd::registry::lint(&dir, ledger.as_ref())
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
            fail_on,
            bus,
        } => cmd::doctor::run(deep, sample, fail_on, &bus).await,
        Command::Scout {
            what,
            timeout,
            connect,
            listen,
            context,
            format,
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
                stored.as_ref().map(|c| c.connect.clone()).unwrap_or_default()
            } else {
                connect
            };
            let listen = if listen.is_empty() {
                stored.as_ref().map(|c| c.listen.clone()).unwrap_or_default()
            } else {
                listen
            };
            let timeout = Duration::from_secs(
                timeout
                    .or_else(|| stored.as_ref().and_then(|c| c.timeout))
                    .unwrap_or(5),
            );
            cmd::scout::run(&what, timeout, &connect, &listen, format).await
        }
    }
}

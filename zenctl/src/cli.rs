//! The clap tree: the derive structs and nothing else that can be helped.
//!
//! Split out of `main.rs` when the crate grew a library (#198/#201). Two
//! reasons, and neither is tidiness:
//!
//! * a `tests/` corpus can only assert that every leaf verb is covered if it
//!   can *walk* the tree, which means the tree has to be importable;
//! * `docs/redesign-2026-07.md` names this file, and #209 names the directory
//!   it becomes (`cli/`, one module per family, with `BusArgs` in
//!   `cli/bus.rs`). This is the first step of that, taken now because the
//!   renderer rewrite has to move every call site anyway.
//!
//! Everything here except `Cli` is `pub(crate)`. `Cli` is public because a
//! test walks the tree through it; the rest staying internal is what keeps
//! the rustdoc surface — which CI lints with `-D warnings` — to what this
//! crate actually publishes. For the same reason the prose here names types
//! in backticks rather than linking them: a public doc must not link a
//! private bound (#213's `88970a5`, and this module tripped it once already).
//!
//! `BusArgs` still carries the resolution policy: five `flag`-then-`env`-then
//! -`context`-then-default ladders, the registry union, and the
//! completion-cache write. Extracting those into pure functions is #209's job;
//! they are deliberately untouched here so this move stays mechanical and
//! reviewable.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use clap_complete::ArgValueCandidates;

use crate::completion;
use crate::input::Source;

// The four `ValueEnum`s below live here rather than beside the code that
// consumes them (#239's neighbour): a `pub` clap tree referencing a type in a
// crate-private `cmd` module is the `private_interfaces` lint, which is
// warn-by-default and therefore fatal under this workspace's `-D warnings`.
// They are clap types; this is where clap types live.

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Pattern {
    Steady,
    Jitter,
    Burst,
    Ramp,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum ScoutWhat {
    Router,
    Peer,
    Client,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum ExportAs {
    /// Registry TOML — round-trippable through `SliceSet::from_dirs`.
    Toml,
    /// A JSON Schema bundle built from the producers' served `describe`
    /// replies (RFC 08 §7).
    Jsonschema,
    /// An AsyncAPI 3.0 document: channels from subjects, operations from
    /// procedures.
    Asyncapi,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum PubSource {
    /// The echo/export row shape, one JSON object per stdin line.
    Ndjson,
}

/// How output is rendered: which format, and whether it may carry colour.
///
/// One struct rather than two loose flags, and flattened everywhere either is
/// wanted, so that **adding a `--format` without a `--color` beside it is not
/// something you can do by forgetting** — the same move the `Render` trait
/// makes for the `Json`/`Ndjson` collapse. Before this there were five
/// separate `format` fields and it would have been five separate omissions.
#[derive(Args, Clone, Copy)]
pub(crate) struct OutputArgs {
    /// Output format: table for humans, json (one document) or ndjson (one
    /// object per row) for scripts; auto = table on a tty, ndjson piped.
    #[arg(long, env = "ZENCTL_FORMAT", value_enum, default_value_t = crate::render::Format::Auto)]
    pub(crate) format: crate::render::Format,
    /// Colour: auto (a terminal gets it, a pipe does not), always, never.
    ///
    /// Colour only ever re-encodes a distinction the plain text already makes,
    /// so stripping every escape leaves the same information. `NO_COLOR` and
    /// `CLICOLOR_FORCE` are honoured under `auto`, and `--format json|ndjson`
    /// never carries an escape whatever this says.
    #[arg(long, value_enum, default_value_t = crate::render::ColorChoice::Auto)]
    pub(crate) color: crate::render::ColorChoice,
}

#[derive(Parser)]
#[command(
    name = "zenctl",
    about = "Explore a keyspace-v2 Zenoh bus (RFC 08 §6)",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
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
        #[arg(long, value_name = "TEXT|@FILE|-")]
        body: Option<Source>,
        /// Ship the body verbatim and print payloads as hex; no decode.
        #[arg(long)]
        raw: bool,
        /// Decode the type name, print the payload as hex.
        #[arg(long)]
        hex: bool,
        /// Per-reply line template — the `topic echo` % vocabulary
        /// (%k %K %o %c %p %s %t %v %e %l %n %a %{a.b.c}).
        #[arg(long, value_name = "TEMPLATE")]
        fmt: Option<String>,
        /// Skip schema decode; render structurally.
        #[arg(long)]
        no_decode: bool,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Keyexpr algebra, no session (RFC 03 §4's footguns, diagnosed).
    #[command(subcommand)]
    Key(KeyCmd),
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
    /// Dump a producer's served payload schemas (`@rpc/<producer>/describe`),
    /// or check a payload against one (`schema check`, #159).
    ///
    /// RFC 08 §7: the shapes are served data. A producer that serves no
    /// `describe` degrades honestly — it is not an error.
    #[command(args_conflicts_with_subcommands = true)]
    Schema {
        #[command(subcommand)]
        cmd: Option<SchemaCmd>,
        /// Producer name, e.g. `sysinfo`.
        #[arg(add = ArgValueCandidates::new(completion::producers))]
        producer: Option<String>,
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
    /// Stand up a mock queryable: answer every query on a keyexpr with one
    /// static body, and log every ask (#121).
    ///
    /// The log doubles as a "who is querying this key" probe. Deliberately
    /// no reply scripting — static and file bodies cover the dev-loop case;
    /// the shell covers dynamic replies by restarting serve. (nuze and zsak
    /// own the embedded-language lane, at the cost of a Nushell dependency
    /// and a linked libpython respectively.)
    Serve {
        /// Key expression to serve (full wire form; wildcards welcome).
        #[arg(add = ArgValueCandidates::new(completion::keys))]
        keyexpr: String,
        /// Reply body: inline text, `@file`, or `-` for stdin (read once) —
        /// through the same encode ladder as `topic pub`.
        reply: Source,
        /// Wire encoding to declare on replies. Defaults to the registry's
        /// declared encoding when the keyexpr refines, else none.
        #[arg(long)]
        encoding: Option<String>,
        /// Do not refuse a body the served schema rejects.
        #[arg(long)]
        no_validate: bool,
        /// Reply the bytes verbatim: no schema lookup, no refusal.
        #[arg(long)]
        raw: bool,
        /// Declare the queryable complete — a claim this responder holds
        /// ALL the data the expression names. Say it only when you mean it.
        #[arg(long)]
        complete: bool,
        /// Exit after N queries (0 = until ctrl-c).
        #[arg(long, default_value_t = 0)]
        count: usize,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Cutover acceptance, half one (RFC 09 §6): assert a retired key
    /// family SILENT while the new plane carries traffic.
    ///
    /// Three verdicts, three exits: 0 = old silent AND new speaking; 1 =
    /// the old family still speaks; 2 = everything was quiet — a
    /// non-verdict, because a dead fleet passes the silence half for free.
    /// The leak check's meaning is stated, not inferred: anything outside
    /// `<base>/v1/` that is not the old root.
    Cutover {
        /// The retired key family (a full wire key expression).
        #[arg(long = "old-root", value_name = "KEYEXPR")]
        old_root: String,
        /// Listening window, seconds.
        #[arg(long, default_value_t = 30)]
        window: u64,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Generate registry-driven test traffic (#162): every declared subject
    /// of a producer, schema-synthesized payloads, declared QoS,
    /// class-conscious rates — a mock producer for testing consumers.
    ///
    /// The full plan prints BEFORE anything is published; every sample
    /// carries the RFC 09 §5.3 synthetic marker
    /// ({"synthetic":true,"tool":…,"origin":…}), so a doctor listen window
    /// or a capture can tell this traffic from real. Events stay inside
    /// their declared rate budget on write-once keys. A run wider than 10
    /// subjects needs --i-know.
    Gen {
        /// Only this producer's subjects.
        #[arg(long, add = ArgValueCandidates::new(completion::producers))]
        producer: Option<String>,
        /// Only subjects whose declared path contains this.
        #[arg(long)]
        subject: Option<String>,
        /// Value for a `{var}` in a declared path (repeatable, k=v).
        /// Unnamed vars get deterministic synthetic values, stated in the
        /// plan.
        #[arg(long = "var", value_name = "K=V")]
        vars: Vec<String>,
        /// Origin the generated keys claim (h-<12 hex>). Default: derived
        /// from this session's zid — printed either way, and stamped into
        /// the marker.
        #[arg(long)]
        origin: Option<String>,
        /// Override every entry's rate (Hz). Default: registry-driven —
        /// telemetry 1 Hz, state refreshes at ttl/2, events inside their
        /// declared budget.
        #[arg(long, value_name = "HZ")]
        rate: Option<f64>,
        /// Send-timing shape (all deterministic under --seed).
        #[arg(long, value_enum, default_value = "steady")]
        pattern: Pattern,
        /// Run length, seconds.
        #[arg(long, default_value_t = 10.0)]
        duration: f64,
        /// Synthesis/jitter seed — same seed, same run.
        #[arg(long, default_value_t = 42)]
        seed: u64,
        /// SchemaSet JSON document (RFC 08 §7) for payload shapes when the
        /// bus serves no describe (the registry carries type names, not
        /// shapes).
        #[arg(long, value_name = "FILE")]
        schema_set: Option<PathBuf>,
        /// Also answer introspect (and describe, with --schema-set) for the
        /// impersonated producers — a complete mock producer, not just a
        /// firehose.
        #[arg(long)]
        serve_describe: bool,
        /// Print the plan and publish nothing.
        #[arg(long)]
        dry_run: bool,
        /// Acknowledge a run wider than 10 subjects.
        #[arg(long = "i-know")]
        i_know: bool,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Await an expectation on the bus, exit-coded for CI (#160).
    ///
    /// The subscriber is declared BEFORE the window opens (not-asked is not
    /// "no", RFC 09 §5.1 O4). Exit 0 = met; 1 = not met on a clean
    /// observation; 2 = the observation cannot carry the claim (drops under
    /// a completeness claim, session failure) — the cutover/probe exit
    /// discipline. `--absent` is legitimate ONLY because of that 2.
    ///
    /// CI recipes should run isolated per RFC 09 §0: multicast scouting
    /// off, gossip on, explicit endpoints — a test that scouts is not
    /// isolated, and the contamination flows both ways.
    Expect {
        /// Selector to watch (full wire form — this session is
        /// un-namespaced, RFC 09 §5).
        selector: String,
        /// Observation window, seconds.
        #[arg(long, default_value_t = 30.0)]
        within: f64,
        /// Require at least N samples (default 1 unless --absent).
        #[arg(long)]
        count: Option<u64>,
        /// Samples/second floor, measured over the full window.
        #[arg(long, value_name = "HZ")]
        rate_min: Option<f64>,
        /// Samples/second ceiling, measured over the full window.
        #[arg(long, value_name = "HZ")]
        rate_max: Option<f64>,
        /// Require every observed payload to validate against its served
        /// schema (#159). "Unknowable" fails the assertion, with the reason.
        #[arg(long)]
        valid_payload: bool,
        /// Require observed wire QoS to match: `declared` (each subject's
        /// registry profile) or one profile name for everything.
        #[arg(long, value_name = "declared|PROFILE")]
        qos: Option<String>,
        /// Assert silence: no sample may match within the window.
        #[arg(long, conflicts_with_all = ["count", "rate_min", "rate_max", "valid_payload", "qos"])]
        absent: bool,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Cutover acceptance, half two (RFC 09 §6): a consumer-shaped,
    /// CONCRETE-KEY probe.
    ///
    /// "A probe MUST build its keys the way the product builds them" — a
    /// `*`-origin probe cannot catch a broken origin path. An origin id is
    /// called directly; a hostname resolves through the RFC 06 §6 identity
    /// bridge first, and the probe FAILS if the bridge yields nothing.
    Probe {
        /// Origin id (`h-…`) or hostname (resolved via the bridge).
        target: String,
        /// Producer name.
        #[arg(add = ArgValueCandidates::new(completion::producers))]
        producer: String,
        /// Procedure path, e.g. `introspect`.
        #[arg(add = ArgValueCandidates::new(completion::procedures))]
        procedure: String,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Capture a selector's traffic to a .zrec file (RFC 09 §5.2).
    ///
    /// Through the Monitor: a bus that outruns the disk surfaces as drop
    /// records *in the file*, where the gaps happened (RFC 09 §5.1 O6) —
    /// a capture is a bounded observer and says what it cost. Replay with
    /// `zenctl replay`; the file is ndjson (one row per line, payloads
    /// lossless as base64 `bytes`), so `jq` reads it too.
    Record {
        /// Full wire selector to capture. Defaults to all v1 data under
        /// the base: `<base>/v1/**` — which `**` being unable to cross
        /// `@`-chunks makes media-safe, and blind to the verbatim planes.
        #[arg(add = ArgValueCandidates::new(completion::keys))]
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
        /// Output file.
        #[arg(long, short = 'o', value_name = "FILE")]
        out: String,
        /// Stop after this many seconds.
        #[arg(long, value_name = "SECS")]
        duration: Option<u64>,
        /// Stop after this many samples (0 = until ctrl-c or --duration).
        #[arg(long, default_value_t = 0)]
        count: u64,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Replay a .zrec capture onto the bus — replay is PUBLISHING.
    ///
    /// Real puts through declared publishers, at the capture's own pacing
    /// (scaled by --speed), re-stamped with this session's HLC: re-stamped
    /// old data WINS last-writer-wins against a live fleet, which is why
    /// the etiquette is enforced (RFC 09 §5.2) — dry-run first, and the
    /// capture header's base is a contract (`--force-base` to override).
    Replay {
        /// The .zrec file to replay.
        file: String,
        /// Pacing scale: 2.0 replays twice as fast as captured.
        #[arg(long, default_value_t = 1.0)]
        speed: f64,
        /// List every would-be put and publish nothing (no session is
        /// even opened). Preview pacing is not simulated.
        #[arg(long)]
        dry_run: bool,
        /// Replay even though the resolved base differs from the capture
        /// header's.
        #[arg(long)]
        force_base: bool,
        /// Replay recorded deletes that fall off the state class — the
        /// same operator price as `topic retire` (RFC 04 §1.2, v1.12).
        #[arg(long = "i-know")]
        i_know: bool,
        /// QoS profile for rows that recorded none.
        #[arg(long, default_value = "refreshed",
              add = ArgValueCandidates::new(completion::qos_profiles))]
        qos: String,
        #[command(flatten)]
        bus: BusArgs,
    },
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
        what: Vec<ScoutWhat>,
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
        #[command(flatten)]
        out: OutputArgs,
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
        /// Listen passively to the data planes for this many seconds after
        /// the GET fan-in and judge what rides (#161): undecodable/invalid
        /// payloads, declared-vs-observed QoS, unregistered traffic,
        /// over-rate events. The report states the window, its scopes, and
        /// what the bounded observer dropped (O5/O6).
        //
        // `--listen-for`, not `--listen`, because `-l/--listen` is the
        // endpoint flag on all 51 verbs (#239). Two arguments of one name
        // made `doctor --help` panic in a debug build and swallowed `-l` in
        // a release one, so `doctor` was the single verb that could not join
        // a bus by listening. The `id` is spelled out too: clap derives it
        // from the *field* name, so renaming only the long option leaves the
        // ids equal and the assert still fires.
        #[arg(id = "listen_for", long = "listen-for", value_name = "SECS")]
        listen: Option<f64>,
        /// Exit 1 when a finding at (or above) this severity exists.
        /// Default: always exit 0 — findings are output, not verdicts.
        #[arg(long, value_enum, value_name = "SEVERITY")]
        fail_on: Option<FailOn>,
        #[command(flatten)]
        bus: BusArgs,
    },
}

#[derive(Subcommand)]
pub(crate) enum KeyCmd {
    /// Does `a` include every key `b` can name? Exit 0 yes / 1 no / 2 invalid.
    Includes {
        a: String,
        b: String,
        #[command(flatten)]
        out: OutputArgs,
    },
    /// Can `a` and `b` name a common key? Exit 0 yes / 1 no / 2 invalid.
    ///
    /// When a 'no' is the convention's doing — `**` never crosses an
    /// `@`-chunk (RFC 03 §4 D2), `*` never matches a verbatim service
    /// origin (D4) — the output says so, with the citation.
    Intersects {
        a: String,
        b: String,
        #[command(flatten)]
        out: OutputArgs,
    },
    /// Canonicalize an expression, or print its parse error verbatim.
    Canon {
        expr: String,
        #[command(flatten)]
        out: OutputArgs,
    },
}

/// The `--fail-on` severity ceiling (scripting hook; opt-in).
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum FailOn {
    /// Fail on error-severity findings only.
    Error,
    /// Fail on warnings or errors.
    Warning,
}

#[derive(Subcommand)]
pub(crate) enum CacheCmd {
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
pub(crate) enum BenchCmd {
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
pub(crate) enum SchemaCmd {
    /// Validate one payload against its schema — no bus write, exit-coded
    /// for CI (#159): 0 = valid, 1 = does not conform, 2 = could not check.
    ///
    /// The schema comes from a live `describe` (give --producer) or an
    /// offline SchemaSet JSON document (--schema-set; the registry TOMLs
    /// carry type *names*, not shapes, so a directory alone cannot check).
    Check {
        /// Type name to validate against.
        #[arg(long = "type", value_name = "TYPE", add = ArgValueCandidates::new(completion::types))]
        type_name: String,
        /// Payload: inline text, `@file`, or `-` for stdin.
        #[arg(long, value_name = "TEXT|@FILE|-")]
        from: Source,
        /// Producer whose served `describe` carries the schema (live mode).
        #[arg(long, add = ArgValueCandidates::new(completion::producers))]
        producer: Option<String>,
        /// SchemaSet JSON document (RFC 08 §7) — offline mode, no session.
        #[arg(long, value_name = "FILE")]
        schema_set: Option<std::path::PathBuf>,
        /// Wire encoding of the payload. Defaults to the schema kind's own
        /// (json-schema → JSON, protobuf → protobuf, cdr → XCDR1).
        #[arg(long)]
        encoding: Option<String>,
        #[command(flatten)]
        bus: BusArgs,
    },
}

#[derive(Subcommand)]
pub(crate) enum RegistryCmd {
    /// Export the loaded slice set as a document.
    ///
    /// `--as toml` round-trips through `--registry <dir>`; `--as jsonschema`
    /// bundles the producers' served `describe` schemas (RFC 08 §7);
    /// `--as asyncapi` maps subjects to channels and procedures to operations.
    Export {
        /// Output document. A foreign schema, so `--format` has no say over
        /// it: passing both is a usage error, not a silent preference.
        // #243. Enforced in `refuse_foreign_format` rather than by
        // `conflicts_with`, which would fire on `ZENCTL_FORMAT` too.
        #[arg(long = "as", value_enum, default_value = "toml")]
        target: ExportAs,
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
    /// Write or update the RFC 08 §3.1 compatibility lock (registry.lock).
    ///
    /// Additive evolution and `[[deprecated]]` retirement regenerate cleanly;
    /// an INCOMPATIBLE edit (changed type/class/kind/shape on an existing
    /// path) is refused — retire and add a sibling instead. `--force`
    /// overrides, and prints every broken pin: the escape hatch is legal,
    /// silent it is not.
    Lock {
        /// The registry directory (the one a build script points at).
        dir: PathBuf,
        /// Rewrite pins over an incompatible edit — the loud break.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum AdminCmd {
    // `admin get` was folded into the first-class `zenctl get` (#114): the
    // generic browse lost its misleading name, and the new verb keeps the
    // admin space reachable (`zenctl get '@/**'`) with strictly more honest
    // rendering (error envelopes, typed payloads, non-lossy bytes).
    /// Enumerate routers/peers (zid, version, locators).
    Routers {
        #[command(flatten)]
        bus: BusArgs,
    },
    /// The mesh as the admin space answers it (#118): nodes, edges, and
    /// who only got mentioned.
    ///
    /// Their pictures are unlabeled circles; ours says which of admin space
    /// and liveliness backs each element. Nodes whose admin space is off
    /// render "heard of, not queryable" — never omitted.
    Graph {
        /// Emit Graphviz instead of the table (pipe to `dot -Tsvg`).
        ///
        /// A foreign schema, so `--format` has no say over it: passing both is
        /// a usage error, not a silent preference.
        // #243, and see `refuse_foreign_format` for why not `conflicts_with`.
        #[arg(long)]
        dot: bool,
        /// Also join liveliness origins to their sessions (#131) — one
        /// extra admin sweep; attachments come from the admin sources or
        /// they are shown as merely reported, never guessed.
        #[arg(long)]
        origins: bool,
        #[command(flatten)]
        bus: BusArgs,
    },
}

#[derive(Subcommand)]
pub(crate) enum StorageCmd {
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
pub(crate) enum BlobCmd {
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
    /// Ask every origin who holds an object, with a *tiny* reply
    /// (RFC 07 §2.5, total across tiers since v1.17): `have`/`manifest` for
    /// an artifact, `store/<algo>/have` for a chunk, `tree/<root>/have` for
    /// a snapshot — at data-low, never the bytes.
    ///
    /// There is no `--origin`: fanning out is what a probe *is*.
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
pub(crate) enum ContextCmd {
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
        /// Zenoh JSON5 config file for this context (#122) — the
        /// passthrough that reaches a secured bus; explorer knobs apply
        /// on top (flag > env > context > file).
        #[arg(long, value_name = "FILE")]
        zenoh_config: Option<PathBuf>,
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
pub(crate) enum TopicCmd {
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
        /// %l payload bytes, %n counter, %T timestamp, %a attachment (empty
        /// when none arrived), %q QoS axes (priority/congestion/reliability,
        /// +express — the wire's actual axes, #120), %S source zid:eid#sn
        /// (empty when the publisher attaches no SourceInfo), %{a.b.c} a
        /// decoded payload field by dot-path, %% literal percent.
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
    // The two shapes — `<KEY> <BODY>` or `--from ndjson` — are the parser's
    // business, not the dispatch's (#209): exactly one source is required, and
    // a key without a body is a usage error rather than an `anyhow` message
    // four frames later. That moves the refusal's exit code from 1 to 2, which
    // is what clap exits with for every other mis-shaped invocation here.
    // Deliberately a `//` comment: it is an argument about the parser, not
    // help text, and `///` would print it under `topic pub --help`.
    #[command(group(clap::ArgGroup::new("source").required(true).args(["from", "key"])))]
    Pub {
        /// Full wire key to publish on (omit with --from ndjson).
        #[arg(requires = "body")]
        key: Option<String>,
        /// Payload: inline text, `@file`, or `-` for stdin (omit with --from).
        body: Option<Source>,
        /// Read rows from stdin instead: `--from ndjson` accepts the exact
        /// row shape `topic echo --format ndjson` (and the zengui export)
        /// emits — key + value per row, optionally encoding/qos/delete/
        /// attachment. One shape, both directions; malformed rows are
        /// counted and reported, never silently skipped.
        #[arg(long, value_enum)]
        from: Option<PubSource>,
        /// With --from: delete rows on keys that are not state-shaped are
        /// refused (and counted) unless this is passed — RFC 04 §1.2
        /// (v1.12) prices the off-state tombstone even in a pipe.
        #[arg(long = "i-know")]
        i_know: bool,
        /// QoS profile (RFC 04 §3): sampled|refreshed|transition|alert|frame.
        /// Defaults to the subject's declared profile when the key refines
        /// against a loaded registry, else `sampled` — either way the choice
        /// and its source are printed (#158).
        #[arg(long, add = ArgValueCandidates::new(completion::qos_profiles))]
        qos: Option<String>,
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
        /// Attachment riding beside the payload: inline text, `@file`, or
        /// `-` for stdin. Never schema-encoded — the registry's vocabulary
        /// ends at the payload (#117).
        #[arg(long, value_name = "TEXT|@FILE|-")]
        attachment: Option<Source>,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Retire a key with a tombstone — an authoritative delete
    /// (RFC 04 §1.2), riding a declared publisher like `topic pub` (P7).
    ///
    /// State keys retire freely (retirement is the class's own semantics);
    /// anything else is an operator cleanup (v1.12) and needs --i-know.
    /// Wildcards are refused outright.
    Retire {
        /// Full wire key to retire (concrete — wildcards are refused).
        #[arg(add = ArgValueCandidates::new(completion::keys))]
        key: String,
        /// QoS profile for the tombstone (RFC 04 §3). A retirement is the
        /// final state transition, so it defaults to the reliable profile.
        #[arg(long, default_value = "transition", add = ArgValueCandidates::new(completion::qos_profiles))]
        qos: String,
        /// Retire a key that is not state-shaped — the RFC 04 §1.2 (v1.12)
        /// operator act. The refusal you are overriding names its reason.
        #[arg(long = "i-know")]
        i_know: bool,
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
        /// Also report observed pub→sub latency per key (implies --per-key):
        /// arrival wall-clock minus publisher HLC — contains clock skew, and
        /// is labeled as such; negative values are the skew evidence
        /// (#119). Unstamped samples are counted, never treated as zero.
        #[arg(long)]
        latency: bool,
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
pub(crate) enum NodeCmd {
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
pub(crate) enum BaseCmd {
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
pub(crate) enum ServiceCmd {
    /// List registered procedures (bus-served slices, or `--registry`).
    List {
        /// Only this producer.
        #[arg(long)]
        producer: Option<String>,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// One producer's `@rpc` surface, with the key shape a call would use.
    ///
    /// `service list` says what exists across the fleet; this says what one
    /// producer offers and how to reach it — the question left over, and the
    /// one you have immediately before `service call`.
    Info {
        /// Producer name.
        #[arg(add = ArgValueCandidates::new(completion::producers))]
        producer: String,
        /// Only this procedure path, e.g. `introspect`.
        #[arg(add = ArgValueCandidates::new(completion::procedures))]
        procedure: Option<String>,
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
        /// Request body: inline JSON, `@file`, or `-` for stdin.
        #[arg(long, value_name = "TEXT|@FILE|-")]
        body: Option<Source>,
        /// Attachment riding beside the request, verbatim — never
        /// schema-encoded (#117's rule, on the call side: #126). Inline
        /// text, `@file`, or `-` for stdin.
        #[arg(long, value_name = "TEXT|@FILE|-")]
        attachment: Option<Source>,
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
pub(crate) enum InterfaceCmd {
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
pub(crate) struct BusArgs {
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
    pub(crate) base: Option<String>,
    /// Use a named context from the config file for this invocation
    /// (default: the file's `current` pointer; env `ZENCTL_CONTEXT`).
    #[arg(long, value_name = "NAME", add = ArgValueCandidates::new(completion::contexts))]
    pub(crate) context: Option<String>,
    /// Local registry directory (`registry/*.toml`), repeatable. When given,
    /// registry-sourced commands answer offline from these files instead of
    /// the live bus.
    #[arg(long, value_name = "DIR")]
    pub(crate) registry: Vec<PathBuf>,
    /// Endpoint to connect to, repeatable (e.g. `tcp/127.0.0.1:7447`).
    #[arg(long, short = 'c')]
    pub(crate) connect: Vec<String>,
    /// Endpoint to listen on, repeatable.
    #[arg(long, short = 'l')]
    pub(crate) listen: Vec<String>,
    /// Enable multicast scouting.
    ///
    /// OFF by default, and you should think before turning it on: a scouting
    /// explorer joins whatever mesh it can find, which is how a throwaway
    /// session ends up talking to a production fleet.
    #[arg(long)]
    pub(crate) scouting: bool,
    /// Seconds to wait for replies / to watch the bus (default 5; a
    /// context may override the default).
    #[arg(long)]
    pub(crate) timeout: Option<u64>,
    /// Zenoh JSON5 config file (#122): the passthrough that reaches a
    /// secured bus (TLS, QUIC with certs, usrpwd, …). Loaded as the base
    /// layer; --connect/--listen/--scouting apply on top when given
    /// (flag > env > context > file). A file that sets a session namespace
    /// is refused — explorers run un-namespaced (RFC 09 §5).
    #[arg(long, value_name = "FILE", env = "ZENCTL_ZENOH_CONFIG")]
    pub(crate) zenoh_config: Option<PathBuf>,
    #[command(flatten)]
    pub(crate) out: OutputArgs,
}

/// `--format` selects among **zenkey's own three renderings** of a report. A
/// foreign document format — `--as toml|jsonschema|asyncapi`, `--dot` — is
/// somebody else's schema, so the two are mutually exclusive (#243).
///
/// ## Why this is not `conflicts_with`
///
/// It was, for about ten minutes. `--format` carries `env = "ZENCTL_FORMAT"`,
/// and clap counts an env-sourced value as *present* for conflict purposes —
/// so a shell that exports a format preference could no longer run `admin
/// graph --dot` at all, with no way to unset it for one invocation short of
/// `env -u`. An exported default is a preference, not a request; only what the
/// user typed on this command line can conflict with what else they typed on
/// it.
///
/// Clap will not answer that from a derived struct, so this asks the
/// `ArgMatches` directly, once, at the edge.
pub(crate) fn refuse_foreign_format(matches: &clap::ArgMatches) {
    use clap::CommandFactory as _;
    use clap::parser::ValueSource;

    // Walk to the leaf: `admin graph` and `registry export` are both two deep,
    // and the flags live on the leaf's own matches.
    let mut m = matches;
    while let Some((_, sub)) = m.subcommand() {
        m = sub;
    }
    // `value_source` panics on an id this subcommand does not define, so ask
    // whether it is defined here first — most leaves have neither flag.
    let typed = |id: &str| {
        m.ids().any(|i| i.as_str() == id) && m.value_source(id) == Some(ValueSource::CommandLine)
    };
    if !typed("format") {
        return;
    }
    for (id, flag) in [("target", "--as"), ("dot", "--dot")] {
        if typed(id) {
            // A clap error, not an `anyhow` one: this is a usage error, and
            // usage errors in this tool exit 2 and print a usage line. The
            // only reason it is not a `conflicts_with` attribute is the env
            // var, and that is no reason for it to look different.
            Cli::command()
                .error(
                    clap::error::ErrorKind::ArgumentConflict,
                    format!(
                        "the argument '{flag}' cannot be used with '--format <FORMAT>'\n\n\
                         {flag} emits a foreign document format — somebody else's \
                         schema — while --format chooses among zenctl's own three \
                         renderings of a report. Drop one."
                    ),
                )
                .exit();
        }
    }
}

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
//! ## The shape of the tree (#307)
//!
//! Three kinds of thing, and the depth says which:
//!
//! * a **noun** is something declared, alive or persisted, and gets a family
//!   with verbs under it — `topic`, `node`, `base`, `service`, `interface`,
//!   `schema`, `registry`, `storage`, `blob`, `admin`, `key`;
//! * a **wire verb** is an act or an observation on live traffic, and hangs
//!   off the root — `get`, `echo`, `pub`, `retire`, `rate`, `field`, `record`,
//!   `replay`, `timeline`, `snapshot`, `serve`, `gen`, `scout`;
//! * a **judgement** is exit-coded under the one contract in [`crate::exit`],
//!   and the exit-coded assertions live together under `check`.
//!
//! That is what moved `echo`/`pub`/`retire` out of `topic` (they are not
//! things a registry declares), collapsed `topic hz` and `topic bw` into
//! `rate --bytes`, turned the `schema` noun/verb hybrid into `schema show`,
//! and gathered `expect`/`cutover`/`retired`/`probe`/`schema check` under
//! `check`. No aliases and no shims: the old spellings are gone, and
//! `zenctl/CHANGELOG.md` carries the table.
//!
//! ## The flag vocabulary (#307)
//!
//! One spelling per concept, and every duration is `f64` seconds:
//!
//! * `--for <SECS>` — **every** passive observation window (it replaced
//!   `--window`, `--within`, `--duration`-as-a-window and `--listen-for`);
//! * `--timeout <SECS>` — reply-wait, and nothing else;
//! * `--duration <SECS>` — bounds **generated output**, so only `gen` has it;
//! * `--watch` is a bare bool and `--every <SECS>` is the one period;
//! * `--count <N>` is a stop bound, and only that — `expect` asserts with
//!   `--at-least`, `bench` issues `--calls`, `pub` repeats `--times`;
//! * `--from` names an input *source*, never an origin;
//! * `--i-know` is one guard per verb, and a verb with two guards spells the
//!   second one out (`gen --wide`).
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

/// Which clock `timeline` orders on (#216).
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum OrderArg {
    /// The observer's monotonic clock, µs since the window epoch. Every
    /// sample has one; breaks sit where they fell.
    Arrival,
    /// The sample's HLC. Only stamped samples have one — the rest are
    /// counted as excluded, never defaulted to their arrival time — and
    /// the report states whether one stamper (happened-before) or several
    /// (skewed wall clocks) produced the axis.
    Hlc,
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

/// Where a wire watcher looks: one typed selector, **or** the three grammar
/// positions composed server-side (#307).
///
/// Flattened onto every verb that opens a subscription — `echo`, `rate`,
/// `record`, `field`, `check expect`, `why` — so composition is a property of
/// *watching the bus* rather than of the three verbs that happened to have
/// grown it. Before this, `expect`, `field` and `why` made you hand-write a
/// selector the grammar could have composed.
///
/// The positions are positions, not filters (RFC 03): they are placed in the
/// key expression and resolved by the router, never applied to samples after
/// they arrive. They are also mutually exclusive with a typed selector — a
/// flag silently overridden by a positional is a flag that lied.
#[derive(Args)]
pub(crate) struct SelectorArgs {
    /// Full wire selector to watch — this session is un-namespaced (RFC 09
    /// §5). Defaults to all v1 data under the base: `<base>/v1/**`, which
    /// `**` being unable to cross an `@`-chunk makes media-safe and blind to
    /// the verbatim planes (RFC 03 §4 D2).
    #[arg(add = ArgValueCandidates::new(completion::keys))]
    pub(crate) selector: Option<String>,
    /// Only this origin (`h-…` or `@service`).
    #[arg(long, conflicts_with = "selector")]
    pub(crate) origin: Option<String>,
    /// Only this class: telemetry, state, or events.
    // Parsed at the edge (#351): clap rejects an unknown class with the
    // vocabulary in the message, so no verb re-validates it. A `//` comment,
    // not a doc one — this is a note to us, and a doc comment here is
    // `--help` text.
    #[arg(long, conflicts_with = "selector",
          add = ArgValueCandidates::new(completion::classes))]
    pub(crate) class: Option<zenkey::Class>,
    /// Only this producer.
    #[arg(long, conflicts_with = "selector",
          add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: Option<String>,
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

/// The `gen` verb's flags (#162/#163) — one struct so the verb's whole body,
/// the fault double-guard included, lives in `cmd/generate.rs` (#209's rule:
/// `run()` dispatches, it does not compute).
#[derive(clap::Args)]
pub(crate) struct GenArgs {
    /// Only this producer's subjects.
    #[arg(long, add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: Option<String>,
    /// Only subjects whose declared path contains this.
    #[arg(long)]
    pub(crate) subject: Option<String>,
    /// Value for a `{var}` in a declared path (repeatable, k=v).
    /// Unnamed vars get deterministic synthetic values, stated in the
    /// plan.
    #[arg(long = "var", value_name = "K=V")]
    pub(crate) vars: Vec<String>,
    /// Origin the generated keys claim (h-<12 hex>). Default: derived
    /// from this session's zid — printed either way, and stamped into
    /// the marker.
    #[arg(long)]
    pub(crate) origin: Option<String>,
    /// Override every entry's rate (Hz). Default: registry-driven —
    /// telemetry 1 Hz, state refreshes at ttl/2, events inside their
    /// declared budget.
    #[arg(long, value_name = "HZ")]
    pub(crate) rate: Option<f64>,
    /// Send-timing shape (all deterministic under --seed).
    #[arg(long, value_enum, default_value = "steady")]
    pub(crate) pattern: Pattern,
    /// How long to keep generating, seconds. The one `--duration` in the
    /// tool, and it bounds OUTPUT: a passive window is `--for` everywhere
    /// (#307).
    #[arg(long, value_name = "SECS", default_value_t = 10.0)]
    pub(crate) duration: f64,
    /// Synthesis/jitter seed — same seed, same run.
    #[arg(long, default_value_t = 42)]
    pub(crate) seed: u64,
    /// Inject fault(s) into otherwise-valid samples (#163) for
    /// consumer-robustness testing on a bus you own. Comma-separated
    /// kinds: truncate, wrong-type, extra-field, unregistered-key,
    /// wrong-qos, missing-encoding, unstamped. Each perturbs one dimension
    /// post-synthesis; the plan states the delta per key, and every
    /// faulted sample's marker carries fault=<kind> (RFC 09 §5.3).
    /// DOUBLE-GUARDED: requires --i-know AND an endpoint or --base TYPED on
    /// this command line — an exported ZENCTL_BASE or context default is the
    /// ambient bus the shell was pointed at, which is exactly what faults
    /// must never land on.
    #[arg(long = "fault", value_name = "KIND", value_delimiter = ',')]
    pub(crate) fault: Vec<String>,
    /// SchemaSet JSON document (RFC 08 §7) for payload shapes when the
    /// bus serves no describe (the registry carries type names, not
    /// shapes).
    #[arg(long, value_name = "FILE")]
    pub(crate) schema_set: Option<PathBuf>,
    /// Also answer introspect (and describe, with --schema-set) for the
    /// impersonated producers — a complete mock producer, not just a
    /// firehose.
    #[arg(long)]
    pub(crate) serve_describe: bool,
    /// Print the plan and publish nothing.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Mean the faults: the acknowledging half of --fault's double guard.
    //
    // One `--i-know` per verb (#307). `gen` has two guards — deliberately
    // non-conforming traffic, and a fleet-wide impersonation — and one flag
    // discharging both meant acknowledging the wide run also armed the fault
    // injector. The graver guard keeps the name; the other is `--wide`.
    #[arg(long = "i-know")]
    pub(crate) i_know: bool,
    /// Acknowledge a run wider than 10 subjects — a fleet-wide impersonation.
    #[arg(long)]
    pub(crate) wide: bool,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `why` verb's flags (#214) — one struct, the `GenArgs` pattern, so the
/// ladder's whole body lives in `cmd/why.rs` (#209's rule: `run()`
/// dispatches, it does not compute).
#[derive(clap::Args)]
pub(crate) struct WhyArgs {
    #[command(flatten)]
    pub(crate) selector: SelectorArgs,
    /// Listen passively for this many seconds — the one rung that costs the
    /// data plane (RFC 09 §5.1, the v1.18 frugality note). Without it the
    /// wire-heard rung reads "not asked", never "silent" (O4).
    #[arg(long = "for", value_name = "SECS")]
    pub(crate) for_secs: Option<f64>,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `echo` verb's flags — one struct, the `GenArgs` pattern, because the
/// arm that destructured them inline was the single largest thing in `run()`.
#[derive(clap::Args)]
pub(crate) struct EchoArgs {
    #[command(flatten)]
    pub(crate) selector: SelectorArgs,
    /// kcat-style format string: %k wire key, %K base-relative, %o origin,
    /// %c class, %p producer, %s subject, %t type, %v value, %e encoding,
    /// %l payload bytes, %n counter, %T timestamp, %a attachment (empty
    /// when none arrived), %q QoS axes (priority/congestion/reliability,
    /// +express — the wire's actual axes, #120), %S source zid:eid#sn
    /// (empty when the publisher attaches no SourceInfo), %{a.b.c} a
    /// decoded payload field by dot-path, %% literal percent.
    #[arg(long)]
    pub(crate) fmt: Option<String>,
    /// Print raw payload bytes as hex instead of decoding.
    #[arg(long)]
    pub(crate) raw: bool,
    /// Decode the type tag but show the payload as hex.
    #[arg(long)]
    pub(crate) hex: bool,
    /// Append the live aggregate sample rate to each line.
    #[arg(long)]
    pub(crate) rate: bool,
    /// Skip schema decode (structural rendering only).
    #[arg(long)]
    pub(crate) no_decode: bool,
    /// Stop after this many samples (0 = run until interrupted).
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub(crate) count: usize,
    /// Seed current state before going live (RFC 04 §3.2, issue #92):
    /// subscribe first, then pull publishers' caches and router storages
    /// through one LWW merge; a boundary line marks where the seed ends
    /// and live begins.
    #[arg(long)]
    pub(crate) seed: bool,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `pub` verb's flags (issue #47) — one struct, the `GenArgs` pattern.
// The two shapes — `<KEY> <BODY>` or `--from ndjson` — are the parser's
// business, not the dispatch's (#209): exactly one source is required, and
// a key without a body is a usage error rather than an `anyhow` message
// four frames later. That moves the refusal's exit code from 1 to 2, which
// is what clap exits with for every other mis-shaped invocation here.
// Deliberately a `//` comment: it is an argument about the parser, not
// help text, and `///` would print it under `pub --help`.
#[derive(clap::Args)]
#[command(group(clap::ArgGroup::new("source").required(true).args(["from", "key"])))]
pub(crate) struct PubArgs {
    /// Full wire key to publish on (omit with --from ndjson).
    #[arg(requires = "body", add = ArgValueCandidates::new(completion::keys))]
    pub(crate) key: Option<String>,
    /// Payload: inline text, `@file`, or `-` for stdin (omit with --from).
    pub(crate) body: Option<Source>,
    /// Read rows from stdin instead: `--from ndjson` accepts the exact
    /// row shape `echo --format ndjson` (and the zengui export) emits —
    /// key + value per row, optionally encoding/qos/delete/attachment. One
    /// shape, both directions; malformed rows are counted and reported,
    /// never silently skipped, and echo's tagged meta lines
    /// (`"row":"dropped"`/`"row":"seed"`) are skipped as stream metadata —
    /// counted as skipped, not malformed.
    #[arg(long, value_enum)]
    pub(crate) from: Option<PubSource>,
    /// With --from: delete rows on keys that are not state-shaped are
    /// refused (and counted) unless this is passed — RFC 04 §1.2
    /// (v1.12) prices the off-state tombstone even in a pipe.
    // `conflicts_with = "key"`, not `requires = "from"`: on the
    // positional-key shape there is nothing this flag can acknowledge,
    // and an accepted-but-inert flag is a mis-shape — refused at exit 2
    // like every other one here. (`requires` cannot say it: clap resolves
    // the requirement through the `source` group, so the positional key
    // satisfies it.)
    #[arg(long = "i-know", conflicts_with = "key")]
    pub(crate) i_know: bool,
    /// QoS profile (RFC 04 §3): sampled|refreshed|transition|alert|frame.
    /// Defaults to the subject's declared profile when the key refines
    /// against a loaded registry, else `sampled` — either way the choice
    /// and its source are printed (#158).
    #[arg(long, add = ArgValueCandidates::new(completion::qos_profiles))]
    pub(crate) qos: Option<String>,
    /// Wire encoding to declare (e.g. application/json). Defaults to the
    /// registry's declared encoding when the key refines, else none.
    #[arg(long)]
    pub(crate) encoding: Option<String>,
    /// Publish this many times.
    //
    // `--times`, default 1, not `--repeat` default 0 (#307): a count flag
    // whose 0 and 1 both mean "once" has one spelling too many, and the one
    // that reads as "none" is the one people typed.
    #[arg(long, value_name = "N", default_value_t = 1)]
    pub(crate) times: usize,
    /// Seconds between repeats — the one period flag (#307).
    #[arg(long, value_name = "SECS", default_value_t = 1.0)]
    pub(crate) every: f64,
    /// Do not refuse a body the served schema rejects — it ships as typed,
    /// with a note. (It is still encoded when it *does* encode.)
    #[arg(long)]
    pub(crate) no_validate: bool,
    /// Send the bytes verbatim: no schema lookup, no encoding, no refusal.
    /// The escape hatch for a subject this tool cannot type.
    #[arg(long)]
    pub(crate) raw: bool,
    /// Attachment riding beside the payload: inline text, `@file`, or
    /// `-` for stdin. Never schema-encoded — the registry's vocabulary
    /// ends at the payload (#117).
    #[arg(long, value_name = "TEXT|@FILE|-")]
    pub(crate) attachment: Option<Source>,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// `doctor`'s flags — one struct, the `GenArgs` pattern.
#[derive(clap::Args)]
pub(crate) struct DoctorArgs {
    /// Additionally GET current state to check freshness against each
    /// subject's ttl (RFC 04 §1.2) and judge storage coverage — adds
    /// fleet query load.
    #[arg(long)]
    pub(crate) deep: bool,
    /// With --deep: drain at most N state samples per family — bounds
    /// the sweep's cost, not just its output.
    #[arg(long, value_name = "N", requires = "deep")]
    pub(crate) sample: Option<usize>,
    /// Listen passively to the data planes for this many seconds after the
    /// GET fan-in and judge what rides (#161): undecodable/invalid payloads,
    /// declared-vs-observed QoS, unregistered traffic, over-rate events. The
    /// report states the window, its scopes, and what the bounded observer
    /// dropped (O5/O6).
    //
    // `--for`, the one passive-window spelling (#307). It used to be
    // `--listen-for`, which existed only to dodge `-l/--listen`, the endpoint
    // flag on every verb; `--for` collides with nothing and says what it is.
    #[arg(long = "for", value_name = "SECS")]
    pub(crate) for_secs: Option<f64>,
    /// Exit 1 when a finding at (or above) this severity exists.
    /// Default: always exit 0 — findings are output, not verdicts.
    #[arg(long, value_enum, value_name = "SEVERITY")]
    pub(crate) fail_on: Option<FailOn>,
    /// Re-run the checks on an interval and report CHECK-ID TRANSITIONS as
    /// ndjson (#227): the first run states the baseline (one line per stable
    /// check id, from null), every later run prints only genuine changes —
    /// and a run that fails flips every check to `unobservable`, never
    /// silently to "ok".
    //
    // `--transitions`, not `--watch` (#307): `--watch` is a bare bool that
    // re-renders a *state* on the list verbs, and this emits a stream of
    // *changes*, which is the opposite kind of output. Folding it into
    // `watchdog --rule 'doctor <check-id>'` was the alternative and does not
    // fit: that rule names one check id, and the baseline this prints is one
    // line per check id there is.
    #[arg(long)]
    pub(crate) transitions: bool,
    /// With --transitions: seconds between runs.
    #[arg(
        long,
        value_name = "SECS",
        default_value_t = 10.0,
        requires = "transitions"
    )]
    pub(crate) every: f64,
    /// With --transitions: stop after N runs (default: run until interrupted).
    #[arg(long, value_name = "N", requires = "transitions")]
    pub(crate) count: Option<u64>,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    // ── Nouns: what is declared, alive, or persisted ──────────────────────
    /// Subjects: what the registry declares, and what it means.
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
    /// Payload schemas as producers serve them (RFC 08 §7).
    ///
    /// The shapes are served data, not registry data: the TOMLs carry type
    /// *names*. A producer that serves no `describe` degrades honestly — it
    /// is not an error. Validating a payload against one is `check schema`.
    #[command(subcommand)]
    Schema(SchemaCmd),
    /// The registry as a document: export it, diff it, lint it, lock it.
    #[command(subcommand)]
    Registry(RegistryCmd),
    /// Storages: what the mesh persists, joined against declared state —
    /// and the router block that makes it persist (RFC 09 §2).
    #[command(subcommand)]
    Storage(StorageCmd),
    /// Router access control: RFC 09 §3's grant matrix, generated from an
    /// enrollment file (#392).
    #[command(subcommand)]
    Acl(AclCmd),
    /// The `@blob` plane: who serves bulk content, and fetching it (RFC 07 §2).
    #[command(subcommand)]
    Blob(BlobCmd),
    /// Zenoh admin space (`@/**`) — the middleware's own introspection.
    #[command(subcommand)]
    Admin(AdminCmd),
    /// Keyexpr algebra, no session (RFC 03 §4's footguns, diagnosed).
    #[command(subcommand)]
    Key(KeyCmd),
    /// Measure the fleet.
    #[command(subcommand)]
    Bench(BenchCmd),

    // ── Wire verbs: acts and observations on live traffic ─────────────────
    /// GET any selector with the fleet discipline (RFC 05 §2.1).
    ///
    /// Target All, consolidation None, every reply attributed by its own
    /// key; error envelopes render as errors (RFC 05 §3), and payloads ride
    /// the same rendering ladder as `echo` (served-schema decode →
    /// structural → text → hex). `@/**` browses the zenoh admin space.
    /// Exit codes: 0 values only, 1 an error reply, 2 silence.
    Get(GetArgs),
    /// Subscribe and print decoded samples (on-bus).
    ///
    /// With a served `describe` schema (RFC 08 §7) payloads decode into
    /// named fields; otherwise they render structurally, tagged with the
    /// registry-declared type. --origin/--class/--producer compose the
    /// selector server-side — never client-side filtering the grammar can
    /// express by position.
    Echo(EchoArgs),
    /// Publish to a key (issue #47) — a declared publisher, never an ad-hoc
    /// put (P7).
    Pub(PubArgs),
    /// Retire a key with a tombstone — an authoritative delete
    /// (RFC 04 §1.2), riding a declared publisher like `pub` (P7).
    ///
    /// State keys retire freely (retirement is the class's own semantics);
    /// anything else is an operator cleanup (v1.12) and needs --i-know.
    /// Wildcards are refused outright.
    Retire(RetireArgs),
    /// Measure publish rate over a window (ros2-style), or bytes with --bytes.
    ///
    /// One verb, because they are one observation: `topic hz` and `topic bw`
    /// watched the same window through the same Monitor and differed only in
    /// which column the table led with (#307).
    Rate(RateArgs),
    /// Field intelligence over a window (#223): per-dotted-path statistics —
    /// presence, type stability, change count, numeric range, small-domain
    /// values — plus the three findings per-sample validation cannot see.
    ///
    /// The stuck sensor that passes every check: a key publishing at its
    /// declared rate with a perfectly valid payload whose temperature_c has
    /// not moved in four hours. field-stuck flags a numeric unchanged over a
    /// span long relative to the subject's declared ttl_s (an observation
    /// with a stated window, never a verdict); field-vanished a path SEEN
    /// then absent while later payloads still parse (a schema that declares
    /// it optional reads Valid without it by construction); field-new a path
    /// the served schema never declared — schema drift at field granularity.
    /// The path table is bounded and reports what it dropped (RFC 09
    /// §5.1 O6); with no registry loaded, stuck/new are unjudgeable and the
    /// report says so (O4). Findings are output, not verdicts — exit 0
    /// unless you opt in with `--fail-on`.
    Field(FieldArgs),
    /// Capture a selector's traffic to a .zrec file (RFC 09 §5.2).
    ///
    /// Through the Monitor: a bus that outruns the disk surfaces as drop
    /// records *in the file*, where the gaps happened (RFC 09 §5.1 O6) —
    /// a capture is a bounded observer and says what it cost. Replay with
    /// `zenctl replay`; the file is ndjson (one row per line, payloads
    /// lossless as base64 `bytes`), so `jq` reads it too.
    Record(RecordArgs),
    /// Replay a .zrec capture onto the bus — replay is PUBLISHING.
    ///
    /// Real puts through declared publishers, at the capture's own pacing
    /// (scaled by --speed), re-stamped with this session's HLC: re-stamped
    /// old data WINS last-writer-wins against a live fleet, which is why
    /// the etiquette is enforced (RFC 09 §5.2) — dry-run first, and the
    /// capture header's base is a contract (`--force-base` to override).
    Replay(ReplayArgs),
    /// The fleet timeline (#216): one merged ordering of a window's samples,
    /// lanes per origin/producer, the clock stated per report and the
    /// stamper per lane — and DELIBERATELY NO EDGES.
    ///
    /// A line between two lanes would claim a causality no observer on this
    /// bus can establish. What it can say: on the arrival axis, when each
    /// sample was seen here; on the HLC axis (`--order hlc`), the
    /// happened-before of ONE stamping node, or a comparison of skewed wall
    /// clocks when several stamped (RFC 09 §5.1 O7). Unstamped samples get
    /// their own lane on arrival and cannot be placed on the HLC axis at
    /// all; drops and coalescing render as breaks where they fell (O6). The
    /// per-publisher sequence-number lane is reported UNAVAILABLE on this
    /// zenoh, not empty. `--from <FILE>` reads a .zrec through the same
    /// projection, so live and replay render identically.
    Timeline(TimelineArgs),
    /// Take a fleet snapshot to a .zsnap file (RFC 13 §4.4) — or, with
    /// `diff`, compare two.
    ///
    /// One fan-in GET per selector, folded last-writer-wins per key, each
    /// row carrying what the observer could establish: the exact payload,
    /// whose clock stamped it (O7), the registry rung (O2), the three-valued
    /// verdict, and who HOLDS it — `live` (its origin held an alive token
    /// during the collection, and whether the replier was the stamper),
    /// `storage_only` (a value answered, nobody is saying it now), or
    /// `unattributed` (the roster was not asked, or the key names no
    /// origin). A snapshot is collected OVER a span, never at an instant,
    /// and every rendering says so. Read, never replayed: seeding a fleet
    /// from a file is `replay --seed-state`. Exit 0 wrote the file, 2
    /// nobody answered (silence is not a snapshot).
    Snapshot(SnapshotArgs),
    /// Stand up a mock queryable: answer every query on a keyexpr with one
    /// static body, and log every ask (#121).
    ///
    /// The log doubles as a "who is querying this key" probe. Deliberately
    /// no reply scripting — static and file bodies cover the dev-loop case;
    /// the shell covers dynamic replies by restarting serve. (nuze and zsak
    /// own the embedded-language lane, at the cost of a Nushell dependency
    /// and a linked libpython respectively.)
    Serve(ServeArgs),
    /// Generate registry-driven test traffic (#162): every declared subject
    /// of a producer, schema-synthesized payloads, declared QoS,
    /// class-conscious rates — a mock producer for testing consumers.
    ///
    /// The full plan prints BEFORE anything is published; every sample
    /// carries the RFC 09 §5.3 synthetic marker
    /// ({"synthetic":true,"tool":…,"origin":…}), so a doctor listen window
    /// or a capture can tell this traffic from real. Events stay inside
    /// their declared rate budget on write-once keys. A run wider than 10
    /// subjects needs --wide; faults need --i-know.
    Gen(GenArgs),
    /// Listen for raw scouting Hellos: zid, whatami, locators.
    ///
    /// The layer *below* `base list`: no session is opened, so this answers
    /// "is anything out there at all" and "is multicast working on this
    /// segment". It is also the one zenctl verb where multicast is ON by
    /// default — scouting is the point, and a scout only listens for Hellos
    /// and joins nothing.
    Scout(ScoutArgs),

    // ── Judgement: exit-coded, under one contract (`crate::exit`) ─────────
    /// Exit-coded assertions, all on one contract: 0 = clean, 1 = a finding,
    /// 2 = no verdict (the question could not be asked or proven).
    ///
    /// Everything under here reserves its 2 — which is what makes `check
    /// expect --absent` legitimate at all, and what stops a dead bus reading
    /// as a pass. CI recipes should run isolated per RFC 09 §0: multicast
    /// scouting off, gossip on, explicit endpoints — a test that scouts is
    /// not isolated, and the contamination flows both ways.
    #[command(subcommand)]
    Check(CheckCmd),
    /// Check the fleet against the contracts it claims (RFC 08 §6): drift,
    /// freshness, QoS, coverage.
    ///
    /// RFC 08 §6: "A disagreement between introspection and the checked-in TOML
    /// is a finding, not an ambiguity." This prints the findings — `registry
    /// diff` shows the two registries side by side; doctor *judges* the
    /// deployment. The local truth comes from `--registry <dir>`; without it
    /// only the roster-vs-introspect check runs.
    Doctor(DoctorArgs),
    /// Why is this key silent — the non-verdict, itemised (#214).
    ///
    /// A rung ladder over facts the engine already holds: scope reach,
    /// grammar, registry declaration, the liveliness roster, declared
    /// publishers, storage coverage, a stored value, freshness, admin
    /// reachability. Every rung answers established / not-established (with
    /// its reason) / NOT ASKED — "not asked" is never rendered as "no"
    /// (RFC 09 §5.1 O4), because silence is never a verdict (RFC 05 §3.1).
    /// "No publisher declared" never reads as a bug: publishers declare
    /// lazily, on the first publication (RFC 08 §6.1). The default run costs
    /// the control plane only; `--for` adds the one data-plane rung.
    /// Exit 0 = nothing found and everything checked looks healthy; 1 = a
    /// cause was established — the finding; 2 = the observation was impaired.
    Why(WhyArgs),
    /// Watch conditions on the bus and emit TRANSITIONS as ndjson (#227).
    ///
    /// A foreground observer — explicitly launched, one process per
    /// invocation, no shared state; not a daemon. Each --rule is one
    /// condition from a CLOSED vocabulary (no expressions, no templating);
    /// every genuine state change prints one line
    /// {"row":"transition","rule":…,"from":…,"to":…,"at":…,"evidence":…} and
    /// an unchanged tick prints nothing. Three states, not two (RFC 09 §5.1 O4/O6):
    /// ok / firing / unobservable — a drop under a completeness claim is
    /// "could not tell", never "ok", which is what keeps a 3am page honest.
    /// The first evaluation states each rule's baseline once, from null.
    Watchdog(WatchdogArgs),

    // ── Meta ──────────────────────────────────────────────────────────────
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
}

/// The exit-coded assertions (#307). One family, one contract — see
/// `crate::exit`.
#[derive(Subcommand)]
pub(crate) enum CheckCmd {
    /// Await an expectation on the bus, exit-coded for CI (#160).
    ///
    /// The subscriber is declared BEFORE the window opens (not-asked is not
    /// "no", RFC 09 §5.1 O4). Exit 0 = met; 1 = not met on a clean
    /// observation; 2 = the observation cannot carry the claim (drops under
    /// a completeness claim, session failure). `--absent` is legitimate
    /// ONLY because of that 2.
    Expect(CheckExpectArgs),
    /// Cutover acceptance, half one (RFC 09 §6): assert a retired key
    /// family SILENT while the new plane carries traffic.
    ///
    /// Three verdicts, three exits: 0 = old silent AND new speaking; 1 =
    /// the old family still speaks; 2 = everything was quiet — a
    /// non-verdict, because a dead fleet passes the silence half for free.
    /// The leak check's meaning is stated, not inferred: anything outside
    /// `<base>/v1/` that is not the old root.
    Cutover(CheckCutoverArgs),
    /// The deprecation burn-down: which `[[deprecated]]` entries are actually
    /// finished. Exit 0 = every entry passes, 1 = a retired subject still
    /// speaks, 2 = unproven — silence is not a pass (RFC 05 §3.1).
    ///
    /// For each ledger entry of the `--registry` dirs, four facts: still on
    /// the wire (`--for`), still declared active by a served introspect slice
    /// (the RFC 08 §6.1 lie), still subscribed to by any session (admin
    /// space), and whether `replaced_by` carries traffic — the cutover pair,
    /// per entry. Without `--for`, wire facts read "not listened", never
    /// "absent" (RFC 09 §5.1 O4).
    Retired {
        /// Listen passively for this many seconds to hear the retired
        /// families and their replacements. No window = no wire facts.
        #[arg(long = "for", value_name = "SECS")]
        for_secs: Option<f64>,
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
    /// Fanning out is what `blob locate` does; this verb refuses to.
    Probe(CheckProbeArgs),
    /// Validate one payload against its schema — no bus write, exit-coded
    /// for CI (#159): 0 = valid, 1 = does not conform, 2 = could not check.
    ///
    /// The schema comes from a live `describe` (give --producer) or an
    /// offline SchemaSet JSON document (--schema-set; the registry TOMLs
    /// carry type *names*, not shapes, so a directory alone cannot check).
    Schema(CheckSchemaArgs),
}

#[derive(Subcommand)]
pub(crate) enum KeyCmd {
    /// Does `a` include every key `b` can name? Exit 0 yes / 1 no / 2 invalid.
    Includes(KeyIncludesArgs),
    /// Can `a` and `b` name a common key? Exit 0 yes / 1 no / 2 invalid.
    ///
    /// When a 'no' is the convention's doing — `**` never crosses an
    /// `@`-chunk (RFC 03 §4 D2), `*` never matches a verbatim service
    /// origin (D4) — the output says so, with the citation.
    Intersects(KeyIntersectsArgs),
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
    /// it. Exit 1 when any measured reply was an error envelope, 2 when
    /// nobody answered.
    Rpc(BenchRpcArgs),
}

#[derive(Subcommand)]
pub(crate) enum SchemaCmd {
    /// Dump a producer's served payload schemas (`@rpc/<producer>/describe`).
    Show(SchemaShowArgs),
}

#[derive(Subcommand)]
pub(crate) enum RegistryCmd {
    /// Export the loaded slice set as a document.
    ///
    /// `--as toml` round-trips through `--registry <dir>`; `--as jsonschema`
    /// bundles the producers' served `describe` schemas (RFC 08 §7);
    /// `--as asyncapi` maps subjects to channels and procedures to operations.
    Export(RegistryExportArgs),
    /// Diff local `--registry` files against what the fleet serves.
    ///
    /// RFC 08 §6: a disagreement is a finding, not an ambiguity. `doctor`
    /// judges a deployment; this just shows the two registries side by side.
    Diff {
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Run the RFC 08 §5 registry lints on a directory, as a build would.
    Lint(RegistryLintArgs),
    /// Write or update the RFC 08 §3.1 compatibility lock (registry.lock).
    ///
    /// Additive evolution and `[[deprecated]]` retirement regenerate cleanly;
    /// an INCOMPATIBLE edit (changed type/class/kind/shape on an existing
    /// path) is refused — retire and add a sibling instead. `--force`
    /// overrides, and prints every broken pin: the escape hatch is legal,
    /// silent it is not.
    Lock(RegistryLockArgs),
    /// Who declares a reader of a subject (#224): every declared subscriber
    /// and querier the admin space serves, related to the target by key
    /// algebra and ranked, one row per session, joined to the origin its
    /// alive token attaches.
    ///
    /// A declaration is not proof of use, a `**` declaration intersects
    /// everything and is shown as such, and an admin space that does not
    /// answer is *not asked* — never an empty consumer set (RFC 13 §3 O4).
    Consumers(RegistryConsumersArgs),
    /// The blast radius of changing one declared subject (#224): its
    /// consumers, its storage coverage, what else declares on its family,
    /// and its `[[deprecated]]` entry — in one document.
    Impact(RegistryImpactArgs),
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
    Graph(AdminGraphArgs),
}

#[derive(Subcommand)]
pub(crate) enum AclCmd {
    /// Generate the router's `access_control` block from an enrollment
    /// file — CN ↔ role ↔ origin, one `[[principal]]` each (RFC 09 §3).
    ///
    /// Four facts of zenoh ACL shape every rule, and each is a way a
    /// hand-written block fails silently: matching is keyexpr INCLUSION and
    /// `**` never crosses `@rpc`/`@media`/`@blob` (one rule per plane); `*`
    /// never covers `@catalog` (its own rule); rules alone are refused —
    /// subjects and policies are required; under default deny every
    /// DECLARATION needs allowing too. A fifth, from the reference
    /// deployment: a consumer's declares are checked on egress toward the
    /// publisher's face, so every publishing policy carries a shared
    /// egress-only `interest-prop` rule. With `--registry`, the planes are
    /// narrowed to what host producers declare and `no-remote-actions`
    /// denies exactly the declared write procedures; without one the plan
    /// says what it could not narrow. Field names are zenoh 1.10's
    /// (zenoh-config-1.10.0/src/lib.rs). Exit 1 when a principal was
    /// refused.
    ///
    /// The enrollment file:
    ///
    ///   base = "zensight"                 # optional; default --base
    ///   [fleet]
    ///   catalog_adv = true                # spell @catalog/**/@adv/**
    ///   salt = "zensight-host-id-v1"      # for machine_id → origin (RFC 06 §1)
    ///   [[principal]]
    ///   cn = "h-3fa9c2d41b7e"             # the certificate CN
    ///   role = "host"                     # host | catalog | console | desired-author | watch
    ///   origin = "h-3fa9c2d41b7e"         # or machine_id = "<32 hex>"; both must agree
    ///   adv = true                        # @adv sidecars
    ///   blob_seed = true                  # seeds the router @blob store
    ///   media = true                      # publishes @media
    ///   [[principal]]
    ///   cn = "zensight-console"
    ///   role = "console"
    ///   remote_actions = false            # true drops the no-remote-actions deny
    // Verbatim, so the enrollment example above keeps its lines: clap would
    // otherwise fold it into one.
    #[command(verbatim_doc_comment)]
    Gen(AclGenArgs),
}

#[derive(Subcommand)]
pub(crate) enum StorageCmd {
    /// List configured storages and judge declared state families against
    /// them (covered / partial / uncovered — RFC 04 §4, issue #14).
    List(StorageListArgs),
    /// Plan the router's storages from the registry and a small deployment file, and emit the `plugins.storage_manager` block with `garbage_collection.lifespan` DERIVED (RFC 09 §2, #393)
    ///
    /// RFC 09 §2 specifies class-driven storages, each with a selector, a
    /// literal `strip_prefix` and a `garbage_collection.lifespan` that must be
    /// ≥ the longest `ttl_s` in the registry (§2.3) — and the registry knows
    /// that number. The deployment file names only what the registry cannot:
    /// the base, the volumes (RFC 09 §2.1's capability pair is per volume —
    /// one volume per history mode from the same plugin) and, per storage,
    /// its class and volume. Everything else is derived: the selector from
    /// the class (`@catalog` explicit, because `*` never matches it), the
    /// `strip_prefix` as the literal leftmost run, and the lifespan as
    /// ceil(max covered ttl_s × gc_margin), with the computation shown.
    ///
    /// The plan refuses what the router would refuse — replication on an
    /// all-mode volume (§2.2), a volume nobody declared, a class the registry
    /// declares nothing under — and warns where a caveat applies: overlapping
    /// selectors (§2), `complete = true` off the replicated latest storage
    /// (§2.2), retention that is the database's and not zenoh's (§2.3),
    /// redb's mandatory retention in all mode (§2.1), a seed on a volatile
    /// volume (§2.1). Without a registry the lifespans fall back to the
    /// default and say so; they are not invented.
    ///
    /// Four ways out. The plan report (`--format` as everywhere); `--json5`,
    /// the zenohd block with every derivation and warning as a comment beside
    /// the storage it concerns; `--check`, an exit-coded comparison with what
    /// a live router runs (0 as planned, 1 a difference, 2 no verdict); and
    /// `--explain <key>`, which planned storage takes a key and why. `gen`
    /// alone is an act: exit 0, or 2 when every storage was refused.
    ///
    /// The deployment file, in full:
    ///
    ///   base = "zensight"              # optional; default = --base / context / ""
    ///
    ///   [volumes.fs]                   # id; `backend` is emitted when it differs
    ///   plugin = "fs"                  # memory | fs | rocksdb | influxdb | redb | other
    ///   # history = "latest"           # fixed by the plugin; per volume for redb
    ///   dir = "/var/lib/zenoh/fs"      # any other key passes through verbatim
    ///
    ///   [storages.latest]
    ///   class = "state"                # state | telemetry | events | catalog | catalog-pdns
    ///   # selector = "v1/*/state/sysinfo/**"   # instead of class: a base-relative override
    ///   volume = "fs"
    ///   replication = true             # or { interval = 10.0, … } (RFC 09 §2.2)
    ///   complete = true                # honoured only where §2.2 allows it
    ///   params = { dir = "latest" }    # merged into `volume: { id: "fs", … }`
    ///   # retention = { … }            # the backend's own block (redb), verbatim
    ///   # gc_period_s = 30             # garbage_collection.period
    ///   # gc_margin = 2.0              # lifespan = ceil(max ttl_s × margin)
    ///   # gc_lifespan_s = 86400        # an explicit lifespan; warned about when below max ttl_s
    #[command(verbatim_doc_comment)]
    Gen(StorageGenArgs),
}

#[derive(Subcommand)]
pub(crate) enum BlobCmd {
    /// Which producers declare which `@blob` tiers (registry only, no bus
    /// traffic). A declaration is a capability, never possession.
    List(BlobListArgs),
    /// Ask every origin who holds an object, with a *tiny* reply
    /// (RFC 07 §2.5, total across tiers since v1.17): `have`/`manifest` for
    /// an artifact, `store/<algo>/have` for a chunk, `tree/<root>/have` for
    /// a snapshot — at data-low, never the bytes.
    ///
    /// There is no `--origin`: fanning out is what finding a holder *is*.
    //
    // `locate`, not `probe` (#307): the top-level `check probe` FORBIDS
    // fan-out by rule ("a `*`-origin probe cannot catch a broken origin
    // path"), and this verb IS a fan-out. One word cannot mean both.
    Locate {
        /// `<id>`, `artifact/<id>`, `tree/<hex>` or `store/<algo>/<hex>`.
        target: String,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Fetch from **one** origin's concrete key, at data-low, verifying every
    /// reply against the content root before it reaches disk (RFC 07 §2.1).
    Fetch(BlobFetchArgs),
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
        #[arg(long, value_name = "SECS")]
        timeout: Option<u64>,
        /// Zenoh JSON5 config file for this context (#122) — the
        /// passthrough that reaches a secured bus; explorer knobs apply
        /// on top (flag > env > context > file).
        #[arg(long, value_name = "FILE")]
        zenoh_config: Option<PathBuf>,
        /// Select it as the current context.
        #[arg(long)]
        select: bool,
        #[command(flatten)]
        out: OutputArgs,
    },
    /// List contexts (the `*` marks the current one).
    List {
        #[command(flatten)]
        out: OutputArgs,
    },
    /// Show one context (default: the current one).
    ///
    /// `--format json` is how a CI job asks which deployment a runner is
    /// pinned to: `zenctl context show --format json | jq -r .base`.
    Show {
        name: Option<String>,
        #[command(flatten)]
        out: OutputArgs,
    },
    /// Select the current context.
    Select {
        name: String,
        #[command(flatten)]
        out: OutputArgs,
    },
    /// Remove a context.
    Rm {
        name: String,
        #[command(flatten)]
        out: OutputArgs,
    },
    /// Open the whole config file in $VISUAL/$EDITOR, validating afterwards.
    Edit {
        #[command(flatten)]
        out: OutputArgs,
    },
}

#[derive(Subcommand)]
pub(crate) enum TopicCmd {
    /// List registered subjects.
    ///
    /// Reads each producer's served introspect slice off the live bus by
    /// default — so it works against *any* keyspace-v2 fleet (RFC 08 §6).
    /// With `--registry <dir>` it answers offline from local registry TOMLs.
    List(TopicListArgs),
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
}

#[derive(Subcommand)]
pub(crate) enum NodeCmd {
    /// One node's full story: producers, versions, capabilities, freshness
    /// (issue #49; RFC 08 §6's capability-and-version inventory, per node).
    Info {
        /// The origin id (`h-<12hex>`). A hostname is refused — resolve it
        /// through the catalog first (RFC 06 §6).
        origin: String,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// List live producers from the liveliness roster (on-bus).
    List(NodeListArgs),
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
    List(BaseListArgs),
}

#[derive(Subcommand)]
pub(crate) enum ServiceCmd {
    /// List registered procedures (bus-served slices, or `--registry`).
    List {
        /// Only this producer.
        #[arg(long, add = ArgValueCandidates::new(completion::producers))]
        producer: Option<String>,
        #[command(flatten)]
        bus: BusArgs,
    },
    /// One producer's `@rpc` surface, with the key shape a call would use.
    ///
    /// `service list` says what exists across the fleet; this says what one
    /// producer offers and how to reach it — the question left over, and the
    /// one you have immediately before `service call`.
    Info(ServiceInfoArgs),
    /// Call a procedure (on-bus).
    Call(ServiceCallArgs),
}

#[derive(Subcommand)]
pub(crate) enum InterfaceCmd {
    /// List every payload type the registry slices declare.
    List {
        #[command(flatten)]
        bus: BusArgs,
    },
    /// Show one payload type and every subject that carries it.
    Show(InterfaceShowArgs),
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
    /// Local registry directory (`registry/*.toml`), repeatable. Joined with
    /// the live bus as a union (RFC 08 §6.1): a producer's served slice wins,
    /// these files fill the gaps, and a disagreement is reported — never
    /// silently overwritten. With the bus unreachable they answer alone.
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
    /// Seconds to wait for replies (default 5; a context may override the
    /// default). Reply-wait only: a passive observation window is `--for`.
    #[arg(long, value_name = "SECS")]
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
/// foreign document format — `--as toml|jsonschema|asyncapi`, `--dot`,
/// `--json5` — is somebody else's schema, so the two are mutually exclusive
/// (#243).
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
    for (id, flag) in [("target", "--as"), ("dot", "--dot"), ("json5", "--json5")] {
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

/// `--format json` promises **one document**, and a stream never has one —
/// `echo`, `serve`, `watchdog` and `doctor --transitions` emit rows as they
/// happen (bounded runs included: `echo --count N` is N rows, not a
/// document). Answering ndjson to a request for json is a silent lie, so a
/// *typed* `--format json` on a streaming verb is refused here, at the same
/// edge and on the same terms as [`refuse_foreign_format`]: an exported
/// `ZENCTL_FORMAT=json` is a preference, not a request, and falls back to
/// rows exactly as `auto` piped would.
pub(crate) fn refuse_stream_json(matches: &clap::ArgMatches) {
    use clap::CommandFactory as _;
    use clap::parser::ValueSource;

    let mut m = matches;
    let mut path: Vec<&str> = Vec::new();
    while let Some((name, sub)) = m.subcommand() {
        path.push(name);
        m = sub;
    }
    let streaming = match path.as_slice() {
        ["echo"] | ["serve"] | ["watchdog"] => true,
        // Plain `doctor` is a report and renders json honestly; only the
        // transition stream cannot.
        ["doctor"] => m.get_flag("transitions"),
        _ => false,
    };
    if !streaming {
        return;
    }
    let typed_json = m.ids().any(|i| i.as_str() == "format")
        && m.value_source("format") == Some(ValueSource::CommandLine)
        && m.get_one::<crate::render::Format>("format") == Some(&crate::render::Format::Json);
    if typed_json {
        Cli::command()
            .error(
                clap::error::ErrorKind::InvalidValue,
                "`--format json` promises one document, and a stream has no single \
                 document to emit — use `--format ndjson` (one object per line).",
            )
            .exit();
    }
}

/// Whether the bus target was **typed on this command line** — the fact
/// `gen --fault`'s second guard needs (#163). By the time the derive struct
/// exists, clap has folded `ZENCTL_BASE` into `--base`, and an exported env
/// var is exactly "whatever bus the shell was pointed at": the ambient
/// default the guard refuses. Same `ValueSource` question as
/// [`refuse_foreign_format`], asked at the same edge.
pub(crate) fn gen_target_typed(matches: &clap::ArgMatches) -> bool {
    use clap::parser::ValueSource;

    let mut m = matches;
    while let Some((_, sub)) = m.subcommand() {
        m = sub;
    }
    ["base", "connect", "listen", "zenoh_config"]
        .into_iter()
        .any(|id| {
            m.ids().any(|i| i.as_str() == id)
                && m.value_source(id) == Some(ValueSource::CommandLine)
        })
}

/// The `get` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct GetArgs {
    /// Any key expression, params included (`key?k=v`).
    #[arg(add = ArgValueCandidates::new(completion::keys))]
    pub(crate) selector: String,
    /// Query body: inline text, `@file`, or `-` for stdin — rides the
    /// same encode ladder as `pub` when the selector's key part refines
    /// to a registered subject.
    #[arg(long, value_name = "TEXT|@FILE|-")]
    pub(crate) body: Option<Source>,
    /// Ship the body verbatim and print payloads as hex; no decode.
    #[arg(long)]
    pub(crate) raw: bool,
    /// Decode the type name, print the payload as hex.
    #[arg(long)]
    pub(crate) hex: bool,
    /// Per-reply line template — the `echo` % vocabulary
    /// (%k %K %o %c %p %s %t %v %e %l %n %a %{a.b.c}). %T renders `-`
    /// and %q/%S render empty here: a reply carries no arrival stamp,
    /// QoS axes or SourceInfo — those are subscription-side facts (#120).
    #[arg(long, value_name = "TEMPLATE")]
    pub(crate) fmt: Option<String>,
    /// Skip schema decode; render structurally.
    #[arg(long)]
    pub(crate) no_decode: bool,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `retire` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct RetireArgs {
    /// Full wire key to retire (concrete — wildcards are refused).
    #[arg(add = ArgValueCandidates::new(completion::keys))]
    pub(crate) key: String,
    /// QoS profile for the tombstone (RFC 04 §3). A retirement is the
    /// final state transition, so it defaults to the reliable profile.
    #[arg(long, default_value = "transition", add = ArgValueCandidates::new(completion::qos_profiles))]
    pub(crate) qos: String,
    /// Retire a key that is not state-shaped — the RFC 04 §1.2 (v1.12)
    /// operator act. The refusal you are overriding names its reason.
    #[arg(long = "i-know")]
    pub(crate) i_know: bool,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `rate` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct RateArgs {
    #[command(flatten)]
    pub(crate) selector: SelectorArgs,
    /// Measurement window, seconds.
    #[arg(long = "for", value_name = "SECS", default_value_t = 10.0)]
    pub(crate) for_secs: f64,
    /// Lead with payload bandwidth instead of sample rate.
    #[arg(long)]
    pub(crate) bytes: bool,
    /// Report each concrete key separately.
    #[arg(long)]
    pub(crate) per_key: bool,
    /// Also report source-sequence gaps (needs publishers that attach
    /// SourceInfo; absent info reads as zero, honestly labeled).
    #[arg(long)]
    pub(crate) loss: bool,
    /// Also report observed pub→sub latency per key (implies --per-key):
    /// arrival wall-clock minus publisher HLC — contains clock skew, and
    /// is labeled as such; negative values are the skew evidence
    /// (#119). Unstamped samples are counted, never treated as zero.
    #[arg(long)]
    pub(crate) latency: bool,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `field` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct FieldArgs {
    #[command(flatten)]
    pub(crate) selector: SelectorArgs,
    /// Observation window, seconds.
    #[arg(long = "for", value_name = "SECS", default_value_t = 30.0)]
    pub(crate) for_secs: f64,
    /// Bound on the per-path table, across every key the window sees.
    #[arg(long, value_name = "N", default_value_t = 512)]
    pub(crate) max_paths: usize,
    /// Exit 1 when a finding at (or above) this severity exists —
    /// `doctor`'s opt-in, on the verb that grew the same findings (#307).
    /// Default: always exit 0.
    #[arg(long, value_enum, value_name = "SEVERITY")]
    pub(crate) fail_on: Option<FailOn>,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `record` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct RecordArgs {
    #[command(flatten)]
    pub(crate) selector: SelectorArgs,
    /// Output file.
    #[arg(long, short = 'o', value_name = "FILE")]
    pub(crate) out: String,
    /// Stop after this many seconds. With --on: give up waiting for a
    /// rule after this long (nothing is written; a rule not firing is
    /// not a finding).
    #[arg(long = "for", value_name = "SECS")]
    pub(crate) for_secs: Option<f64>,
    /// Stop after this many samples (0 = until ctrl-c or --for). With
    /// --on: the post-roll's stop bound.
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub(crate) count: u64,
    /// Arm instead of record (#218): write a file only when this rule
    /// transitions to `firing`, with `--pre` seconds of retained traffic
    /// before it. Repeatable; the watchdog's vocabulary — `rate-above
    /// <SEL> <HZ>`, `rate-below <SEL> <HZ>`, `silent-for <SEL> <SECS>`,
    /// `invalid-payload <SEL>`, `qos-mismatch <SEL>`, `doctor <CHECK-ID>`,
    /// `origin-down <ORIGIN>`, `dropped`. The file is `.zrec` version 2
    /// (RFC 13 §4.1): a state preamble, the pre-roll, the trigger record
    /// where it fired, then `--post` seconds more.
    #[arg(long, value_name = "RULE", requires = "pre")]
    pub(crate) on: Vec<String>,
    /// Seconds of traffic to retain before the trigger — the ring's age
    /// budget. The pre-roll covers only the watched selectors (O5), and
    /// the header says how much of it the ring could give (O6).
    #[arg(long, value_name = "SECS", requires = "on")]
    pub(crate) pre: Option<f64>,
    /// Seconds to keep recording after the trigger.
    #[arg(long, value_name = "SECS", default_value_t = 10.0, requires = "on")]
    pub(crate) post: f64,
    /// Seconds between rule evaluations — the one period flag (#307).
    #[arg(long, value_name = "SECS", default_value_t = 1.0, requires = "on")]
    pub(crate) every: f64,
    /// What the state preamble is a snapshot of (RFC 13 §4.3): the
    /// current state of only the keys the ring cannot show, the full
    /// current state under the watched selectors, or none at all.
    #[arg(
        long,
        value_enum,
        default_value = "absent-from-window",
        requires = "on"
    )]
    pub(crate) preamble: PreambleMode,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// `record --preamble`: what a triggered capture's state preamble is a
/// snapshot of — the RFC 13 §4.3 semantics plus "none" (#218).
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum PreambleMode {
    /// Only the state keys absent from the retained window, fetched at
    /// trigger time: what the ring cannot tell you.
    AbsentFromWindow,
    /// Every state key under the watched selectors, fetched at trigger
    /// time, ring or no ring.
    Full,
    /// No preamble — the pre-roll's `state` rows are then deltas with no
    /// base, and the file says nothing to the contrary.
    None,
}

/// The `timeline` verb's flags (#216) — one struct, the `GenArgs` pattern.
#[derive(clap::Args)]
pub(crate) struct TimelineArgs {
    /// Full wire selectors to watch, one lane set per window — this session
    /// is un-namespaced (RFC 09 §5). `**` never crosses an `@`-chunk, so a
    /// `v1/**` window excludes the verbatim planes by construction and the
    /// report says so (RFC 03 §4 D2). Not with --from.
    #[arg(value_name = "SELECTOR", required_unless_present = "from",
          add = ArgValueCandidates::new(completion::keys))]
    pub(crate) selectors: Vec<String>,
    /// The passive window, seconds. Not with --from: a capture's span is in
    /// its rows.
    #[arg(long = "for", value_name = "SECS", required_unless_present = "from")]
    pub(crate) for_secs: Option<f64>,
    /// Which clock to order on. Every emitted row carries `order_by`, so a
    /// line cut out of the stream still says which axis its `pos` is on.
    #[arg(long, value_enum, default_value = "arrival")]
    pub(crate) order: OrderArg,
    /// Read the window from a .zrec capture instead of the bus — the same
    /// projection, so the same window rendered live and from its file is
    /// identical. Keys are read under the capture's stated base.
    #[arg(long, value_name = "FILE", conflicts_with_all = ["selectors", "for_secs"])]
    pub(crate) from: Option<PathBuf>,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `replay` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub(crate) struct SnapshotArgs {
    #[command(flatten)]
    pub(crate) selector: SelectorArgs,
    /// Output file. Refused (exit 2) when nobody answered: a file of
    /// silence would read as an empty fleet (RFC 05 §3.1).
    #[arg(long, short = 'o', value_name = "FILE", required = true)]
    pub(crate) out: Option<String>,
    /// Replies kept per selector; past it they are drained, counted, and
    /// the header says how many (RFC 09 §5.1 O6).
    #[arg(long, value_name = "N", default_value_t = zenkey_fleet::DEFAULT_MAX_REPLIES)]
    pub(crate) max_replies: usize,
    /// Skip the liveliness roster. Cheaper, and every holder is then
    /// `unattributed` — the file says so rather than guessing.
    #[arg(long)]
    pub(crate) no_roster: bool,
    #[command(subcommand)]
    pub(crate) cmd: Option<SnapshotSub>,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

#[derive(Subcommand)]
pub(crate) enum SnapshotSub {
    /// Compare two .zsnap files — no bus. Both spans are stated, the
    /// facets (value, verdict, registration, holder) stay apart, and an
    /// origin an alignment could not pair is listed, never dropped.
    /// Exit 0 identical, 1 they differ, 2 a file could not be read.
    Diff(SnapshotDiffArgs),
}

#[derive(clap::Args)]
pub(crate) struct SnapshotDiffArgs {
    /// The earlier snapshot.
    pub(crate) a: String,
    /// The later snapshot.
    pub(crate) b: String,
    /// Align origins across deployments by the labels their state
    /// documents carry (chunk DD; not implemented in this build).
    #[arg(long)]
    pub(crate) normalize_origins: bool,
    /// An explicit origin pairing, `A=B`, repeatable (chunk DD; not
    /// implemented in this build).
    #[arg(long = "map", value_name = "A=B")]
    pub(crate) maps: Vec<String>,
    /// Field-level changes listed per key before the rest are counted.
    #[arg(long, value_name = "N", default_value_t = 20)]
    pub(crate) max_changes: usize,
    #[command(flatten)]
    pub(crate) out: OutputArgs,
}

#[derive(clap::Args)]
pub(crate) struct ReplayArgs {
    /// The .zrec file to replay.
    pub(crate) file: String,
    /// Pacing scale: 2.0 replays twice as fast as captured.
    #[arg(long, default_value_t = 1.0)]
    pub(crate) speed: f64,
    /// List every would-be put and publish nothing (no session is
    /// even opened). Preview pacing is not simulated.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Replay even though the resolved base differs from the capture
    /// header's.
    #[arg(long)]
    pub(crate) force_base: bool,
    /// Replay recorded deletes that fall off the state class — the
    /// same operator price as `retire` (RFC 04 §1.2, v1.12).
    #[arg(long = "i-know")]
    pub(crate) i_know: bool,
    /// Publish a version-2 capture's preamble rows too — state at capture
    /// start, re-stamped now (RFC 13 §4.1). Off by default: re-stamped
    /// state wins last-writer-wins, so the preamble republishes a whole
    /// snapshot over the live fleet with no pacing between the rows
    /// (§4.2); the rows are skipped and counted instead.
    #[arg(long)]
    pub(crate) seed_state: bool,
    /// QoS profile for rows that recorded none.
    #[arg(long, default_value = "refreshed",
          add = ArgValueCandidates::new(completion::qos_profiles))]
    pub(crate) qos: String,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `serve` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct ServeArgs {
    /// Key expression to serve (full wire form; wildcards welcome).
    #[arg(add = ArgValueCandidates::new(completion::keys))]
    pub(crate) keyexpr: String,
    /// Reply body: inline text, `@file`, or `-` for stdin (read once) —
    /// through the same encode ladder as `pub`.
    pub(crate) reply: Source,
    /// Wire encoding to declare on replies. Defaults to the registry's
    /// declared encoding when the keyexpr refines, else none.
    #[arg(long)]
    pub(crate) encoding: Option<String>,
    /// Do not refuse a body the served schema rejects.
    #[arg(long)]
    pub(crate) no_validate: bool,
    /// Reply the bytes verbatim: no schema lookup, no refusal.
    #[arg(long)]
    pub(crate) raw: bool,
    /// Declare the queryable complete — a claim this responder holds
    /// ALL the data the expression names. Say it only when you mean it.
    #[arg(long)]
    pub(crate) complete: bool,
    /// Exit after N queries (0 = until ctrl-c).
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub(crate) count: usize,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `scout` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct ScoutArgs {
    /// Filter by advertised kind. Repeatable; default: all three.
    #[arg(long, value_enum)]
    pub(crate) what: Vec<ScoutWhat>,
    /// Seconds to wait for Hellos (default 5; a context may override).
    #[arg(long, value_name = "SECS")]
    pub(crate) timeout: Option<u64>,
    /// Endpoint to connect to, repeatable — reaches gossip scouting
    /// where multicast is filtered.
    #[arg(long, short = 'c')]
    pub(crate) connect: Vec<String>,
    /// Endpoint to listen on, repeatable.
    #[arg(long, short = 'l')]
    pub(crate) listen: Vec<String>,
    /// Use a named context for endpoints/timeout defaults.
    #[arg(long, value_name = "NAME", add = ArgValueCandidates::new(completion::contexts))]
    pub(crate) context: Option<String>,
    /// table = census (deduped by zid); ndjson = arrival log, one Hello
    /// per line as heard.
    #[command(flatten)]
    pub(crate) out: OutputArgs,
}

/// The `watchdog` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct WatchdogArgs {
    /// One rule (repeatable): `rate-above <SEL> <HZ>`,
    /// `rate-below <SEL> <HZ>`, `silent-for <SEL> <SECS>`,
    /// `invalid-payload <SEL>`, `qos-mismatch <SEL>`,
    /// `doctor <CHECK-ID>`, `origin-down <ORIGIN>`, `dropped`.
    /// Selectors are full wire form (this session is un-namespaced,
    /// RFC 09 §5); a doctor rule runs the doctor once per tick.
    #[arg(long = "rule", value_name = "RULE", required = true)]
    pub(crate) rules: Vec<String>,
    /// Seconds between evaluations — the one period flag (#307).
    #[arg(long, value_name = "SECS", default_value_t = 5.0)]
    pub(crate) every: f64,
    /// Stop after N evaluations (default: run until interrupted).
    #[arg(long, value_name = "N")]
    pub(crate) count: Option<u64>,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `check expect` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct CheckExpectArgs {
    #[command(flatten)]
    pub(crate) selector: SelectorArgs,
    /// Observation window, seconds.
    #[arg(long = "for", value_name = "SECS", default_value_t = 30.0)]
    pub(crate) for_secs: f64,
    /// Require at least N samples (default 1 unless --absent).
    //
    // `--at-least`, not `--count` (#307): `--count` is a stop bound
    // everywhere else in the tool, and here it was an assertion — the
    // one flag name that meant the opposite of itself.
    #[arg(long, value_name = "N")]
    pub(crate) at_least: Option<u64>,
    /// Samples/second floor, measured over the full window.
    #[arg(long, value_name = "HZ")]
    pub(crate) rate_min: Option<f64>,
    /// Samples/second ceiling, measured over the full window.
    #[arg(long, value_name = "HZ")]
    pub(crate) rate_max: Option<f64>,
    /// Require every observed payload to validate against its served
    /// schema (#159). "Unknowable" fails the assertion, with the reason.
    #[arg(long)]
    pub(crate) valid_payload: bool,
    /// Require observed wire QoS to match: `declared` (each subject's
    /// registry profile) or one profile name for everything.
    #[arg(long, value_name = "declared|PROFILE",
          add = ArgValueCandidates::new(completion::qos_profiles))]
    pub(crate) qos: Option<String>,
    /// Assert silence: no sample may match within the window.
    #[arg(long, conflicts_with_all = ["at_least", "rate_min", "rate_max", "valid_payload", "qos"])]
    pub(crate) absent: bool,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `check cutover` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct CheckCutoverArgs {
    /// The retired key family (a full wire key expression).
    #[arg(long = "old-root", value_name = "KEYEXPR")]
    pub(crate) old_root: String,
    /// Listening window, seconds.
    #[arg(long = "for", value_name = "SECS", default_value_t = 30.0)]
    pub(crate) for_secs: f64,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `check probe` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct CheckProbeArgs {
    /// Origin id (`h-…`) or hostname (resolved via the bridge).
    pub(crate) target: String,
    /// Producer name.
    #[arg(add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: String,
    /// Procedure path, e.g. `introspect`.
    #[arg(add = ArgValueCandidates::new(completion::procedures))]
    pub(crate) procedure: String,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `check schema` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct CheckSchemaArgs {
    /// Type name to validate against.
    #[arg(long = "type", value_name = "TYPE", add = ArgValueCandidates::new(completion::types))]
    pub(crate) type_name: String,
    /// Payload: inline text, `@file`, or `-` for stdin.
    #[arg(long, value_name = "TEXT|@FILE|-")]
    pub(crate) from: Source,
    /// Producer whose served `describe` carries the schema (live mode).
    #[arg(long, add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: Option<String>,
    /// SchemaSet JSON document (RFC 08 §7) — offline mode, no session.
    #[arg(long, value_name = "FILE")]
    pub(crate) schema_set: Option<std::path::PathBuf>,
    /// Wire encoding of the payload. Defaults to the schema kind's own
    /// (json-schema → JSON, protobuf → protobuf, cdr → XCDR1).
    #[arg(long)]
    pub(crate) encoding: Option<String>,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `key includes` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct KeyIncludesArgs {
    pub(crate) a: String,
    pub(crate) b: String,
    #[command(flatten)]
    pub(crate) out: OutputArgs,
}

/// The `key intersects` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct KeyIntersectsArgs {
    pub(crate) a: String,
    pub(crate) b: String,
    #[command(flatten)]
    pub(crate) out: OutputArgs,
}

/// The `bench rpc` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct BenchRpcArgs {
    /// Origin to target: a host id, `*` for the fleet, or `@catalog`.
    pub(crate) origin: String,
    /// Producer name.
    #[arg(add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: String,
    /// Procedure path. `introspect` is the safe default: RFC 08 §6 makes
    /// it a read every producer serves.
    #[arg(default_value = "introspect", add = ArgValueCandidates::new(completion::procedures))]
    pub(crate) procedure: String,
    /// Calls to issue (default 100).
    //
    // `--calls`, not `--count` (#307): `--count` is a stop bound on a
    // stream everywhere else, and this is the size of the experiment.
    #[arg(long, value_name = "N")]
    pub(crate) calls: Option<usize>,
    /// Calls in flight at once (1 = strictly sequential).
    #[arg(long, default_value_t = 1)]
    pub(crate) concurrency: usize,
    /// Bench a procedure the registry does not declare idempotent.
    #[arg(long = "i-know")]
    pub(crate) i_know: bool,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `schema show` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct SchemaShowArgs {
    /// Producer name, e.g. `sysinfo`.
    #[arg(add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: String,
    /// Show only this type (implies the full document).
    #[arg(long = "type", value_name = "TYPE", add = ArgValueCandidates::new(completion::types))]
    pub(crate) type_name: Option<String>,
    /// Print every schema document in full, not just kind + hash.
    #[arg(long)]
    pub(crate) full: bool,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `registry export` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct RegistryExportArgs {
    /// Output document. A foreign schema, so `--format` has no say over
    /// it: passing both is a usage error, not a silent preference.
    // #243. Enforced in `refuse_foreign_format` rather than by
    // `conflicts_with`, which would fire on `ZENCTL_FORMAT` too.
    #[arg(long = "as", value_enum, default_value = "toml")]
    pub(crate) target: ExportAs,
    /// Only this producer.
    #[arg(long, add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: Option<String>,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `registry lint` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct RegistryLintArgs {
    /// The registry directory (the one a build script points at).
    pub(crate) dir: PathBuf,
    /// Deprecation ledger; defaults to `<dir>/deprecated.lock`.
    #[arg(long, value_name = "FILE")]
    pub(crate) ledger: Option<PathBuf>,
    #[command(flatten)]
    pub(crate) out: OutputArgs,
}

/// The `registry lock` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct RegistryLockArgs {
    /// The registry directory (the one a build script points at).
    pub(crate) dir: PathBuf,
    /// Rewrite pins over an incompatible edit — the loud break.
    #[arg(long)]
    pub(crate) force: bool,
    #[command(flatten)]
    pub(crate) out: OutputArgs,
}

/// The `registry consumers` verb's flags — one struct the dispatcher hands
/// over whole, destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct RegistryConsumersArgs {
    /// `<producer>/<subject-path>` as the registry spells it (resolved to the
    /// family's wire selector under the base), or a raw key or selector —
    /// anything carrying `*` or `@`, or starting at the base or `v1/`.
    #[arg(add = ArgValueCandidates::new(completion::keys))]
    pub(crate) target: String,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `registry impact` verb's flags — one struct the dispatcher hands over
/// whole, destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct RegistryImpactArgs {
    /// `<producer>/<subject-path>` as the registry spells it. A path that
    /// survives only in the `[[deprecated]]` ledger still resolves — who
    /// still reads a retired subject is the ledger's own question.
    pub(crate) target: String,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `admin graph` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct AdminGraphArgs {
    /// Emit Graphviz instead of the table (pipe to `dot -Tsvg`).
    ///
    /// A foreign schema, so `--format` has no say over it: passing both is
    /// a usage error, not a silent preference.
    // #243, and see `refuse_foreign_format` for why not `conflicts_with`.
    #[arg(long)]
    pub(crate) dot: bool,
    /// Also join liveliness origins to their sessions (#131) — one
    /// extra admin sweep; attachments come from the admin sources or
    /// they are shown as merely reported, never guessed.
    #[arg(long)]
    pub(crate) origins: bool,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `acl gen` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct AclGenArgs {
    /// The enrollment file (TOML): CN ↔ role ↔ origin, one [[principal]]
    /// each. See `zenctl acl gen --help` for the shape.
    #[arg(long, value_name = "FILE")]
    pub(crate) enrollment: PathBuf,
    /// Emit the router's `access_control` JSON5 block on stdout, a comment
    /// per rule naming its matrix row and its fact — pipe it into the
    /// router config. A foreign schema, so `--format` has no say over it.
    // #243, and see `refuse_foreign_format` for why not `conflicts_with`.
    #[arg(long, conflicts_with_all = ["check", "explain"])]
    pub(crate) json5: bool,
    /// Compare the plan against a router config file (`--against`): missing,
    /// extra and changed rules, subjects and policies, a CN the enrollment
    /// does not know. Exit 0 identical / 1 findings / 2 not asked.
    ///
    /// A file, not the admin space: zenoh 1.10 serves no GET on
    /// `@/<zid>/router/config/**` (it only subscribes to it for runtime
    /// edits), so the running block is not observable from the bus.
    #[arg(long, requires = "against", conflicts_with = "explain")]
    pub(crate) check: bool,
    /// With --check: the router's JSON5 config file, read through zenoh's
    /// own loader so what is compared is what zenohd would run.
    #[arg(long, value_name = "FILE", requires = "check")]
    pub(crate) against: Option<PathBuf>,
    /// Does PRINCIPAL (a subject id or CN) hold MESSAGE on KEY, via which
    /// rules, in which direction? Inclusion by zenoh-keyexpr. Exit 0.
    #[arg(long, num_args = 3, value_names = ["PRINCIPAL", "KEY", "MESSAGE"])]
    pub(crate) explain: Option<Vec<String>>,
    /// Admit `zid = "…"` subjects. Prototyping only: a ZID is not backed by
    /// authentication, and zenoh's own config says so.
    #[arg(long)]
    pub(crate) allow_zid_subjects: bool,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `storage list` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct StorageListArgs {
    /// Re-render on change.
    #[arg(long)]
    pub(crate) watch: bool,
    /// With --watch: seconds between re-renders.
    #[arg(long, value_name = "SECS", default_value_t = 2.0, requires = "watch")]
    pub(crate) every: f64,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `storage gen` verb's flags (#393) — one struct the dispatcher hands
/// over whole, destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct StorageGenArgs {
    /// The deployment file (TOML) — see the long help for its shape.
    #[arg(long, value_name = "FILE")]
    pub(crate) deployment: PathBuf,
    /// Emit the zenohd `plugins.storage_manager` block (JSON5, with every
    /// derivation and warning as a comment beside the storage it concerns)
    /// instead of the plan report.
    ///
    /// A foreign schema, so `--format` has no say over it: passing both is
    /// a usage error, not a silent preference.
    // #243, and see `refuse_foreign_format` for why not `conflicts_with`.
    #[arg(long, conflicts_with_all = ["check", "explain"])]
    pub(crate) json5: bool,
    /// Compare the plan against the storages a live router runs (the admin
    /// space): missing, extra, a differing key_expr / strip_prefix / volume,
    /// a gc.lifespan below the computed minimum. Exit 0 = as planned, 1 = a
    /// difference, 2 = no verdict (the admin space answered nothing, or the
    /// question could not be put).
    #[arg(long, conflicts_with = "explain")]
    pub(crate) check: bool,
    /// Which planned storage(s) would take this key, and why — pure over the
    /// plan, exit 0.
    #[arg(long, value_name = "KEY")]
    pub(crate) explain: Option<String>,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `blob list` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct BlobListArgs {
    /// Only this producer's declarations.
    #[arg(long, add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: Option<String>,
    /// Only this tier: artifact, tree or store.
    #[arg(long, add = ArgValueCandidates::new(completion::blob_tiers))]
    pub(crate) tier: Option<String>,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `blob fetch` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct BlobFetchArgs {
    /// `<id>`, `artifact/<id>`, `tree/<hex>` or `store/<algo>/<hex>`.
    pub(crate) target: String,
    /// The one origin to fetch from (`h-<12hex>` or `@service`) — as
    /// reported by `zenctl blob locate`. A wildcard is not an origin.
    //
    // `--origin`, not `--from` (#307): `--from` names an input *source*
    // in this tool (`pub --from ndjson`, `check schema --from @file`),
    // and an origin is a place on the bus, not a source of bytes to
    // read.
    #[arg(long, value_name = "ORIGIN")]
    pub(crate) origin: String,
    /// Where to write. Defaults to the target's last chunk; the origin's
    /// advisory filename is never used to choose a path.
    #[arg(long, short = 'o', value_name = "PATH")]
    pub(crate) out: Option<PathBuf>,
    /// The content root the reference carried (RFC 07 §2.1). Every reply
    /// is verified against it before disk.
    #[arg(long, value_name = "HEX")]
    pub(crate) root: Option<String>,
    /// Accept whatever this origin serves, without a root to check it
    /// against — trust-on-first-use, stated out loud.
    #[arg(long, conflicts_with = "root")]
    pub(crate) allow_unpinned: bool,
    /// Replace an existing destination file.
    #[arg(long)]
    pub(crate) overwrite: bool,
    /// Suppress progress on stderr.
    #[arg(long, short = 'q')]
    pub(crate) quiet: bool,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `topic list` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct TopicListArgs {
    /// Only this producer.
    #[arg(long, add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: Option<String>,
    /// Only this class: telemetry, state, or events.
    #[arg(long, add = ArgValueCandidates::new(completion::classes))]
    pub(crate) class: Option<zenkey::Class>,
    /// Only subjects carrying this payload type.
    #[arg(long, value_name = "TYPE", add = ArgValueCandidates::new(completion::types))]
    pub(crate) r#type: Option<String>,
    /// Also list retired subjects from each slice's `[[deprecated]]`
    /// ledger (RFC 08 §6: which hosts still serve a deprecated subject).
    #[arg(long)]
    pub(crate) deprecated: bool,
    /// Re-render on change. Appeared and disappeared subjects are marked
    /// for one cycle; ndjson streams one snapshot object per cycle.
    #[arg(long, conflicts_with = "budget")]
    pub(crate) watch: bool,
    /// With --watch: seconds between re-renders.
    #[arg(long, value_name = "SECS", default_value_t = 2.0, requires = "watch")]
    pub(crate) every: f64,
    /// Observe the bus and add a declared-vs-observed key-population
    /// column (#221): distinct keys per `{var}` family, judged per origin
    /// against the declared `cardinality` (RFC 08 §2). Over is a finding;
    /// under is not — a bounded window proves a lower bound, never the
    /// population — and `{path...}` families are exempt and say so.
    #[arg(long)]
    pub(crate) budget: bool,
    /// With --budget: seconds to observe the key population.
    #[arg(
        long = "for",
        value_name = "SECS",
        default_value_t = 10.0,
        requires = "budget"
    )]
    pub(crate) for_secs: f64,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `node list` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct NodeListArgs {
    /// Join each producer against its served introspect slice (app +
    /// registry version).
    #[arg(long)]
    pub(crate) verbose: bool,
    /// Re-render on liveliness events (no polling — the bus pushes the
    /// roster, so there is no `--every` here). Reflects a producer
    /// stopping within one event.
    #[arg(long)]
    pub(crate) watch: bool,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `base list` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct BaseListArgs {
    /// Re-render on change.
    #[arg(long)]
    pub(crate) watch: bool,
    /// With --watch: seconds between re-renders.
    #[arg(long, value_name = "SECS", default_value_t = 2.0, requires = "watch")]
    pub(crate) every: f64,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `service info` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct ServiceInfoArgs {
    /// Producer name.
    #[arg(add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: String,
    /// Only this procedure path, e.g. `introspect`.
    #[arg(add = ArgValueCandidates::new(completion::procedures))]
    pub(crate) procedure: Option<String>,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `service call` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct ServiceCallArgs {
    /// Origin to target: a host id (`h-3fa9c2d41b7e`), `*` for the whole
    /// fleet, or `@catalog` for a service origin.
    pub(crate) origin: String,
    /// Producer name. Omit for a service origin, which has no producer chunk.
    #[arg(add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: String,
    /// Procedure path, e.g. `introspect` or `artifact/status`.
    #[arg(add = ArgValueCandidates::new(completion::procedures))]
    pub(crate) procedure: String,
    /// Selector parameters, repeatable: `--param state=established`.
    #[arg(long = "param", value_name = "K=V")]
    pub(crate) params: Vec<String>,
    /// Request body: inline JSON, `@file`, or `-` for stdin.
    #[arg(long, value_name = "TEXT|@FILE|-")]
    pub(crate) body: Option<Source>,
    /// Attachment riding beside the request, verbatim — never
    /// schema-encoded (#117's rule, on the call side: #126). Inline
    /// text, `@file`, or `-` for stdin.
    #[arg(long, value_name = "TEXT|@FILE|-")]
    pub(crate) attachment: Option<Source>,
    /// Skip the registry lookup (and with it the registry-layer
    /// forbidden-fanout refusal and any body validation).
    #[arg(long)]
    pub(crate) no_validate: bool,
    /// Send the request body verbatim: no schema lookup, no encoding.
    #[arg(long)]
    pub(crate) raw: bool,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `interface show` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct InterfaceShowArgs {
    /// Type name, e.g. `TelemetryPoint`.
    #[arg(add = ArgValueCandidates::new(completion::types))]
    pub(crate) type_name: String,
    /// Also fetch the served schema from every producer that carries it
    /// (RFC 08 §7). Disagreeing hashes are reported as drift.
    #[arg(long)]
    pub(crate) schema: bool,
    /// With --schema, print each schema document in full.
    #[arg(long)]
    pub(crate) full: bool,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

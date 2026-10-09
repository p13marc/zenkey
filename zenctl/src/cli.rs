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
//!   with verbs under it — `service`, `iface`, `schema`, `namespace`,
//!   `config`, `storage`, `acl`, `blob`, `admin`, `key`, `bench`;
//! * a **wire verb** is an act or an observation on live traffic, and hangs
//!   off the root — `get`, `call`, `watch`, `echo`, `pub`, `rate`, `field`,
//!   `record`, `replay`, `timeline`, `snapshot`, `graph`, `compat`, `export`,
//!   `serve`, `gen`, `scout`;
//! * a **judgement** is exit-coded under the one contract in [`crate::exit`],
//!   and the exit-coded assertions live together under `check`.
//!
//! That is what moved `echo`/`pub` out of `topic` (they are not
//! things a registry declares), collapsed `topic hz` and `topic bw` into
//! `rate --bytes`, turned the `schema` noun/verb hybrid into `schema show`,
//! and gathered `expect`/`cutover`/`retired`/`probe`/`schema check` under
//! `check`. No aliases and no shims: the old spellings are gone, and
//! `zenctl/CHANGELOG.md` carries the table.
//!
//! ## zk2's nouns (#612, FJ4)
//!
//! `service list|show`, `iface list|show`, `schema show`, `graph` and a live
//! `compat` are **resolved** verbs: they read zk2's base-relative keys
//! through a session opened *in* the deployment's namespace
//! (`NamespaceArgs`: `--namespace`, with `--base` as its alias and the
//! context file's `base` as its rung), which ends RFC 09 §5's "explorers are
//! never namespaced" for zk2 (decided 2026-10-08). `namespace list` is the
//! raw half: it looks across namespaces, so its session (`SessionArgs`)
//! takes none. They replaced v1's `topic`, `node`, `base`, `interface` and
//! `registry`, and v1's `service list|info` and `schema show <producer>`.
//!
//! ## zk2's acts and reads (#612, FJ5)
//!
//! `call`, `get state` and `watch` are resolved too: each aims at an
//! address (or a pattern), one interface revision and one resource, and
//! goes through the runtime's `Client`/`Fleet` and `Consumer`. `call`
//! replaced v1's `service call`. `get` keeps two forms that cannot be
//! confused: `get <SELECTOR>` is raw (a wire selector, un-namespaced, any
//! bus), and `get state <ADDRESS> <IFACE> <STATE>` is resolved (the
//! owner's current state, or `--last-known <ARCHIVE>`'s). `pub` refuses a
//! zk2 service's own key (P3) and `retire` is gone; `replay --namespace`
//! publishes through a session in the deployment namespace.
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

// ── Names the grammar can spell (#509) ─────────────────────────────────────
//
// A producer, a procedure path, a config resource or group lands in a chunk
// position of a key, and the typed builders in `zenkey::selector` *assert*
// on an illegal chunk — rightly, for the registry constants they were written
// for, where an illegal one is a programmer error. Typed at a shell it is a
// usage error: `service call h-… MyApp foo` panicked with exit 101, outside
// the 0/1/2 contract (`crate::exit`). So every such argument is validated
// here, by the same `is_valid_plain_chunk` the builders and the registry
// linter use, and clap refuses it — exit 2, naming the argument — before a
// session is ever opened. `zenkey`'s assert stays as it is.
//
// The selector positions (`--producer` beside `--origin`/`--class` on the
// wire verbs) are deliberately not here: they compose a key *expression*,
// where `net*` is a legitimate thing to type, and nothing on that path
// asserts.

/// The rule, spelled once for every refusal below.
const CHUNK_RULE: &str =
    "RFC 03 §2: [a-z0-9]([a-z0-9._-]*[a-z0-9])?, lowercase, alphanumeric at both ends";

/// One plain chunk (RFC 03 §2): a producer, a config resource or group.
fn chunk_arg(s: &str) -> Result<String, String> {
    if zenkey::grammar::is_valid_plain_chunk(s) {
        Ok(s.to_string())
    } else {
        Err(format!("not a plain chunk — {CHUNK_RULE}"))
    }
}

/// A procedure path: plain chunks joined by `/` (`artifact/status`).
fn procedure_arg(s: &str) -> Result<String, String> {
    match s
        .split('/')
        .find(|seg| !zenkey::grammar::is_valid_plain_chunk(seg))
    {
        None => Ok(s.to_string()),
        Some(seg) => Err(format!(
            "{seg:?} is not a plain chunk; a procedure path is plain chunks joined by `/` — \
             {CHUNK_RULE}"
        )),
    }
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
    about = "Explore, query and check a keyspace-v2 Zenoh bus",
    // The crate version, then the build (#513): `git describe` from
    // `build.rs`, or `unknown` where that is not a fact — releases are
    // source-only, so the commit is what names a production binary.
    version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("ZENCTL_GIT_DESCRIBE"), ")")
)]
pub struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

/// The `gen` verb's flags (#612, FJ8a) — one struct so the verb's whole
/// body lives in `cmd/generate.rs` (#209's rule: `run()` dispatches, it
/// does not compute).
#[derive(clap::Args)]
pub(crate) struct GenArgs {
    /// The address the mock owner runs at, `<system>/<service>`: yours to
    /// name. One whose instance token is already present is refused
    /// unless --i-know.
    #[arg(value_name = "SYSTEM/SERVICE", value_parser = addr_arg,
          add = ArgValueCandidates::new(completion::services))]
    pub(crate) address: zenkey_model::grammar::Addr,
    /// The interfaces it implements, `<name>.v<major>[@<fingerprint>]`,
    /// each from --contracts or retrieved from the bus (spec §8.4).
    /// Omitted: every contract --contracts loads, one revision each.
    #[arg(value_name = "IFACE[@FP]", value_parser = revision_arg,
          add = ArgValueCandidates::new(completion::ifaces))]
    pub(crate) ifaces: Vec<RevisionSpec>,
    /// The members a templated resource publishes, repeatable:
    /// `<resource>=<member>[,<member>…]`, each member its parameter values
    /// in template order joined by `/` (`bandwidth/{ns}/{iface}=
    /// default/eth0,default/eth1`). A resource named by none gets two
    /// synthetic members (`<param>-1`, `<param>-2`), stated in the plan.
    #[arg(long = "member", value_name = "RESOURCE=MEMBERS")]
    pub(crate) members: Vec<String>,
    /// A role binding, `ROLE=SYSTEM/SERVICE[,…]`, repeatable (R1): a
    /// contract's required role must be bound, or a service does not
    /// start.
    #[arg(long = "bind", value_name = "ROLE=PROVIDERS")]
    pub(crate) binds: Vec<String>,
    /// Override every stream's and state's rate (Hz). Default: 1 Hz for a
    /// stream, a state re-put every 2 s; an event stays within its
    /// declared rate whatever this says (spec §2.6).
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
    /// Print the plan and bring nothing up (no session is opened when
    /// --contracts holds every interface).
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Start beside an instance already running at the address: a second
    /// writer of its keys (P3) and a split-brain on every exclusive
    /// resource (spec §6).
    #[arg(long = "i-know")]
    pub(crate) i_know: bool,
    #[command(flatten)]
    pub(crate) contracts: ContractArgs,
    #[command(flatten)]
    pub(crate) ns: NamespaceArgs,
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
    /// Full wire key to publish on (concrete — wildcards are refused; omit
    /// with --from ndjson).
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
    /// (v1.12) prices the off-state tombstone even in a pipe. A row on a
    /// wildcard key is refused either way.
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
    /// Seconds between the two presence reads split-brain and token-missing
    /// compare (spec §6): a token in both reads lasted. Keep it above the
    /// longest re-mint overlap the deployment allows; an owner SHOULD keep
    /// one below a second.
    #[arg(long, value_name = "SECS", default_value_t = 2.0)]
    pub(crate) grace: f64,
    /// Also ask state-stamp-foreign: GET every owner's state and check whose
    /// clock stamped each reply (spec §4.2 S1–S2). Costs the owners' data
    /// plane, so it is asked only here.
    #[arg(long)]
    pub(crate) deep: bool,
    /// The token count presence-over-budget judges the presence domain
    /// against (spec §8.3: about 10–15k tokens per domain).
    #[arg(long, value_name = "N", default_value_t = zenkey_fleet::DEFAULT_PRESENCE_BUDGET)]
    pub(crate) presence_budget: usize,
    /// Trust every answer from the routers' admin space. Without it, an
    /// answer counts only when it is verifiably a router's own, from a router
    /// this session is connected to: any session can answer under
    /// `@/<zid>/router` (spec §4.2, 0.12). Pass it only when the deployment's
    /// grants deny `@/**` queryables to every principal (§11.1).
    #[arg(long)]
    pub(crate) trust_admin_space: bool,
    /// Ask only this check (repeatable); every other is not asked.
    #[arg(long = "check", value_name = "CHECK-ID", value_parser = check_id,
          conflicts_with = "skip", add = ArgValueCandidates::new(completion::check_ids))]
    pub(crate) checks: Vec<zenkey_fleet::report::CheckId>,
    /// Do not ask this check (repeatable): it reads `not_asked`, which
    /// neither passes nor fails the run.
    #[arg(long, value_name = "CHECK-ID", value_parser = check_id,
          add = ArgValueCandidates::new(completion::check_ids))]
    pub(crate) skip: Vec<zenkey_fleet::report::CheckId>,
    /// The lowest severity whose finding exits 1 (default: warning; an info
    /// finding is worth knowing, not a failure). With no finding at or above
    /// it, a check left unobservable — or an empty scope, no zk2 token
    /// visible — exits 2: it could be hiding one.
    #[arg(long, value_enum, value_name = "SEVERITY")]
    pub(crate) fail_on: Option<FailOn>,
    /// Re-run the checks on an interval and report CHECK-ID TRANSITIONS as
    /// ndjson (#227): the first run states the baseline (one line per check
    /// asked, from null), every later run prints only genuine changes. A
    /// check is `firing` on a finding, `ok` when established clean, and
    /// `unobservable` otherwise — never silently `ok`.
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
    pub(crate) ns: NamespaceArgs,
}

/// A doctor check id, refused with the vocabulary when it is not one.
fn check_id(s: &str) -> Result<zenkey_fleet::report::CheckId, String> {
    use zenkey_fleet::report::CheckId;
    CheckId::parse(s).ok_or_else(|| {
        format!(
            "not a check id; one of: {}",
            CheckId::ALL.map(CheckId::as_str).join(", ")
        )
    })
}

#[derive(Subcommand)]
pub(crate) enum Command {
    // ── Nouns: what is declared, alive, or persisted ──────────────────────
    /// Running services: their instances, interfaces and descriptors.
    ///
    /// zk2's presence plane (spec §8.1, §3.3): every instance holds an
    /// instance token and a token per interface it provides, and serves a
    /// descriptor naming each interface's full contract fingerprint, its
    /// tokenless set and its roles' bindings. `list` and `show` read both
    /// through a session in the deployment's namespace. Calling a service's
    /// operation is `zenctl call`.
    #[command(subcommand)]
    Service(ServiceCmd),
    /// Read and change a producer's live configuration.
    ///
    /// Configuration resources on the `@rpc` plane (RFC 05 §5.1): read the
    /// served schema beside every running value, change one group typed
    /// against it, and drive a confirmed change to its end.
    #[command(subcommand)]
    Config(ConfigCmd),
    /// Interfaces: who provides each, who requires it, and its contract.
    ///
    /// A zk2 interface is a contract, `<name>.v<major>`, and each revision of
    /// it is a fingerprint (spec §9). Providers come from interface tokens
    /// and, for the tokenless set, from descriptors; consumers from the roles
    /// descriptors declare (R3); contracts from `--contracts` or retrieved
    /// from their holders by fingerprint (§8.4).
    #[command(subcommand)]
    Iface(IfaceCmd),
    /// Payload schemas, as a contract's bundle carries them.
    ///
    /// The schema artifacts of one revision (spec §7.1, §9.5): JSON Schema
    /// documents and protobuf descriptor sets, and which resource member
    /// names which type. Offline from `--contracts`, or retrieved from the
    /// bus. Validating a v1 payload against a served schema is `check
    /// schema`.
    #[command(subcommand)]
    Schema(SchemaCmd),
    /// Deployment namespaces in use on the bus (needs no --namespace).
    #[command(subcommand)]
    Namespace(NamespaceCmd),
    /// Storages: what the mesh persists, and the router config behind it.
    ///
    /// What the mesh persists, joined against declared state — and the
    /// router block that makes it persist (RFC 09 §2).
    #[command(subcommand)]
    Storage(StorageCmd),
    /// Router access control, generated from an enrollment and the contracts.
    ///
    /// zk2's grant shapes (spec §11), compiled from an enrollment and the
    /// contracts into a router's `access_control` block (#612).
    #[command(subcommand)]
    Acl(AclCmd),
    /// Bulk content: who serves it, and fetching it.
    ///
    /// The `@blob` plane (RFC 07 §2).
    #[command(subcommand)]
    Blob(BlobCmd),
    /// Zenoh's own introspection: routers, peers and the mesh graph.
    ///
    /// The admin space (`@/**`) — the middleware's own introspection.
    #[command(subcommand)]
    Admin(AdminCmd),
    /// Key-expression algebra, offline: inclusion, intersection, canon.
    ///
    /// No session is opened. RFC 03 §4's footguns, diagnosed.
    #[command(subcommand)]
    Key(KeyCmd),
    /// Measure how a deployment answers its operations.
    #[command(subcommand)]
    Bench(BenchCmd),

    // ── Wire verbs: acts and observations on live traffic ─────────────────
    /// Query a raw selector, or read a zk2 state resource (`get state`).
    ///
    /// Two forms, never confused. `get <SELECTOR>` is RAW: any key
    /// expression, on a session in no namespace, so the selector is the wire
    /// key as it is (`prod/zk2/…`, `v1/…`, `@/**` for the zenoh admin space).
    /// The fleet discipline (RFC 05 §2.1): target All, consolidation None,
    /// every reply attributed by its own key; error envelopes render as
    /// errors, and payloads ride `echo`'s rendering ladder. `get state
    /// <ADDRESS> <IFACE> <STATE>` is RESOLVED: one zk2 state resource of one
    /// owner, read through its contract in the deployment's namespace — see
    /// `zenctl get state --help`. Exit codes, both forms: 0 values only, 1
    /// an error reply, 2 silence.
    Get(GetArgs),
    /// Call one operation of a zk2 service, through its contract.
    ///
    /// The operation is resolved through its contract (the bundle the bus
    /// serves, or `--contracts`), and the request — JSON, or bytes for a raw
    /// type — is encoded as the operation's request type: JSON or CBOR for a
    /// JSON Schema type, protobuf from its JSON form through the bundle's
    /// descriptor set. One address is called on its concrete key (target
    /// BestMatching, consolidation None; spec §5.1 O1), and only an
    /// idempotent operation is retried, after silence (O4). A `*` in the
    /// address or a template parameter not given makes the call a FAN-OUT:
    /// target All, consolidation None, only to an operation declaring
    /// `fanout = "allowed"` — any other is refused before anything is sent
    /// (O2). A value, an envelope and silence are kept apart (O5): silence
    /// is attributed through presence, and "no token visible" is what this
    /// reader could see, since a refused presence read is empty too (§8.1).
    /// A fan-out's envelopes are unattributed (a `reply_err` carries no
    /// key); a replier with no summary, or two, is possibly partial (O6).
    /// Exit codes: 0 a value and no error reply, 1 an envelope (the
    /// finding), 2 silence or a refused input.
    Call(CallArgs),
    /// Subscribe to a zk2 resource and print every sample, decoded.
    ///
    /// A stream, state or event resource of one address or a pattern
    /// (`*/tc`), subscribed through the runtime's consumer (spec §3.2: at
    /// once, without presence), every sample rendered through the contract
    /// (§7.2). A sample put on a wildcard key is discarded by rule (R6) and
    /// counted apart from what this tool lagged behind. Ends at `--count`
    /// samples, after `--for` seconds, or on ctrl-c, with a summary. Exit 0
    /// when a sample arrived, 2 when none did: silence is never a verdict.
    Watch(WatchArgs),
    /// Subscribe and print decoded samples (on-bus).
    ///
    /// With a served `describe` schema (RFC 08 §7) payloads decode into
    /// named fields; otherwise they render structurally, tagged with the
    /// registry-declared type. --origin/--class/--producer compose the
    /// selector server-side — never client-side filtering the grammar can
    /// express by position.
    Echo(EchoArgs),
    /// Publish a value to a key that no zk2 service owns.
    ///
    /// Through a declared publisher, never an ad-hoc put (P7, issue #47). A
    /// key is written only by the service that owns it (P3, spec §6): a
    /// zk2 service's own key is refused (exit 2) — act on a service through
    /// its operations, `zenctl call`. Foreign keys are written as typed.
    Pub(PubArgs),
    /// Measure publish rate over a window (ros2-style), or bytes with --bytes.
    ///
    /// One verb, because they are one observation: `topic hz` and `topic bw`
    /// watched the same window through the same Monitor and differed only in
    /// which column the table led with (#307).
    Rate(RateArgs),
    /// Per-field statistics over a window: stuck, vanished and new fields.
    ///
    /// Field intelligence (#223): per-dotted-path statistics — presence,
    /// type stability, change count, numeric range, small-domain values —
    /// plus the three findings per-sample validation cannot see.
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
    /// Capture wire selectors' traffic to a .zrec file.
    ///
    /// A capture (`.zrec` version 3, the tooling guide's §5), taken through
    /// the Monitor on a session in no namespace, so rows keep their full wire
    /// keys and their payloads lossless (base64 `bytes`), with the QoS axes
    /// they rode with and the stamp they carried, informatively. The header
    /// carries the namespace you stated (`--namespace`, alias `--base`) and
    /// the exact selectors, and names the verbatim chunks none of them reach
    /// (`@stream`, `@state`, `@op`, `@zk`, `@adv`: O5) — a `zk2/**` capture
    /// excludes them, it does not find them empty. A bus that outruns the
    /// disk surfaces as drop records in the file, where the gaps happened
    /// (O6). Replay with `zenctl replay --namespace`; the file is ndjson, so
    /// `jq` reads it too.
    Record(RecordArgs),
    /// Replay a .zrec capture onto the bus (this publishes).
    ///
    /// Replay is PUBLISHING: real puts through declared publishers, at the capture's own pacing
    /// (scaled by --speed), re-stamped with this session's HLC: re-stamped
    /// old data WINS last-writer-wins against a live fleet, which is why
    /// the etiquette is enforced (RFC 09 §5.2) — dry-run first, and the
    /// capture header's base is a contract (`--force-base` to override).
    /// `--namespace` moves every key into a deployment namespace of its own,
    /// through a session in it: a replayer standing in for the owners it
    /// recorded (spike S13). A zk2 service's own key replayed where that
    /// service runs is refused unless --i-know (P3).
    Replay(ReplayArgs),
    /// One merged ordering of a window's samples, a lane per origin.
    ///
    /// The fleet timeline (#216): lanes per origin/producer, the clock
    /// stated per report and the stamper per lane — and DELIBERATELY NO
    /// EDGES. A line between two lanes would claim a causality no observer on this
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
    /// Take a fleet snapshot to a .zsnap file, or compare two with `diff`.
    ///
    /// The snapshot of RFC 13 §4.4: one fan-in GET per selector, folded
    /// last-writer-wins per key, each row carrying what the observer could
    /// establish: the exact payload, whose clock stamped it (O7), the registry
    /// rung (O2), the three-valued verdict, and who HOLDS it — `live` (its
    /// origin held an alive token during the collection, and whether the
    /// replier was the stamper), `storage_only` (a value answered, nobody is
    /// saying it now), or `unattributed` (the roster was not asked, or the key
    /// names no origin). A snapshot is collected OVER a span, never at an
    /// instant, and every rendering says so. Read, never replayed: seeding a
    /// fleet from a file is `replay --seed-state`. Exit 0 wrote the file, 2
    /// nobody answered (silence is not a snapshot).
    Snapshot(SnapshotArgs),
    /// The data-flow graph: each role bound to the providers it selects.
    ///
    /// Read from descriptors and interface tokens only, never inferred from
    /// traffic (spec §3.2 R3): a node per service, and an edge per binding
    /// that selects a provider present now — the edges the runtime itself
    /// computes. A role whose bindings select nothing stays on its node with
    /// no edge; whether that is wrong is `doctor`'s question. An instance
    /// whose descriptor did not answer is listed, because its roles and its
    /// tokenless interfaces are missing from the graph. `--dot` emits
    /// Graphviz (pipe to `dot -Tsvg`).
    Graph(GraphArgs),
    /// Compare two contract revisions: compatible, review or breaking.
    ///
    /// The contract CI's own classifier (spec §9.8, FULL_TRANSITIVE within a
    /// major, both directions), on two revisions each given as an authoring
    /// file (`*.toml`), a bundle file (`*.bundle.json`), or `<iface>[@<fp>]`
    /// — a revision `--contracts` holds, or one a provider's descriptor
    /// names, retrieved from its holder (spec §8.4); a fingerprint prefix is
    /// enough when it names one revision. Exit 0 compatible, 1 review or
    /// breaking (the finding), 2 no verdict: an input that does not read, or
    /// a revision that could not be had.
    Compat(CompatArgs),
    /// Serve the bus and its contract as Prometheus metrics.
    ///
    /// Key series named and united by the registry, and the observer's own
    /// blind spots as first-class series beside them (#228). `zenctl export
    /// --bind 127.0.0.1:9184`. Metrics ABOUT THE BUS AND THE CONTRACT, not a
    /// general exporter: a series exists only where the registry declares the
    /// subject (names and units from `unit`/`kind`, never sniffed from the
    /// leaf; every `{var}` a label; the declared `cardinality` bounds the
    /// population and what it refuses is counted). What every other exporter
    /// hides is exposed by name (RFC 13 §3): `zenkey_observer_dropped_total`,
    /// the four evicted populations (never summed), coalesced and unstamped
    /// samples; a series that stopped keeps its labels and state — evicted,
    /// origin_down, retired — and loses its value, so absence and silence are
    /// different bytes; payload verdicts are three populations, the third
    /// `not_validated`; the selectors watched and the planes `**` cannot reach
    /// ride `zenkey_scope_info`. Killing a producer turns its series
    /// `origin_down`; forcing drops moves the counter and marks the series fed
    /// meanwhile; scraping twice with no traffic is byte-identical. A
    /// foreground observer, explicitly launched, one process per invocation,
    /// sharing nothing, caching no discovery, serving nothing another zenctl
    /// reads — the permitted second kind (`docs/redesign-2026-07.md` §6.1).
    /// REFUSED up front: OTLP, histograms and summaries, push gateways and
    /// remote write — `/metrics` over plain HTTP is the whole surface.
    Export(ExportArgs),
    /// Serve one operation of an interface as a mock owner, and log each call.
    ///
    /// A real zk2 service at the address you name (P3, spec §6): an
    /// instance, a descriptor (its `meta` carrying the synthetic marker) and
    /// tokens, brought up by the runtime in §8.2's order. The operation
    /// answers every call with one fixed reply — JSON encoded as the
    /// contract's response type, as `call` encodes a request; bytes for a
    /// raw type; synthesized when omitted — or refuses it with --refuse.
    /// Every other operation of the interface is answered too, never silent
    /// (O3): an optional one `unavailable`, a required one `internal`. Each
    /// call is logged as it is answered: its key, what it binds, the request
    /// decoded through the bundle, and the metadata the caller claims (O7).
    /// An address whose instance token is already present is refused unless
    /// --i-know. Ends at --count calls, after --for, or on ctrl-c.
    Serve(ServeArgs),
    /// Bring up a mock owner publishing every resource of its contracts.
    ///
    /// Contract-driven (#612, FJ8a): a real zk2 service at the address you
    /// name (P3, spec §6) — an instance, a descriptor (its `meta` carrying
    /// the synthetic marker {"synthetic":true,"tool":…,"seed":…}) and tokens
    /// — publishing every stream, state and event resource of each interface
    /// through the runtime's writers: the contract's QoS and `Encoding`, the
    /// owner's stamp on every state put (S1), a fresh ULID per event (§2.6),
    /// events within their declared rate. Every operation answers each call
    /// with one synthesized response. Payloads are synthesized from the
    /// bundle, deterministic per --seed: JSON Schema values that validate,
    /// protobuf messages with every field filled, raw bytes of the media
    /// type's size class. The full plan prints BEFORE anything is brought up.
    /// An address whose instance token is already present is refused unless
    /// --i-know.
    Gen(GenArgs),
    /// Listen for raw scouting Hellos: zid, whatami, locators.
    ///
    /// The layer *below* `namespace list`: no session is opened, so this answers
    /// "is anything out there at all" and "is multicast working on this
    /// segment". It is also the one zenctl verb where multicast is ON by
    /// default — scouting is the point, and a scout only listens for Hellos
    /// and joins nothing.
    Scout(ScoutArgs),

    // ── Judgement: exit-coded, under one contract (`crate::exit`) ─────────
    /// Exit-coded assertions: 0 clean, 1 a finding, 2 no verdict.
    ///
    /// All on one contract: 0 = clean, 1 = a finding, 2 = no verdict (the
    /// question could not be asked or proven). Everything under here reserves
    /// its 2 — which is what makes `check expect --absent` legitimate at all,
    /// and what stops a dead bus reading as a pass. CI recipes should run
    /// isolated per RFC 09 §0: multicast scouting off, gossip on, explicit
    /// endpoints — a test that scouts is not isolated, and the contamination
    /// flows both ways.
    #[command(subcommand)]
    Check(CheckCmd),
    /// Judge a zk2 deployment against the core: one verdict per check.
    ///
    /// Thirteen checks, each a question whose finding is the yes: split-brain
    /// (§6), binding-unsatisfied (§3.2), contract-drift (§9.8),
    /// contract-unavailable (§8.4), descriptor-invalid (§3.3), token-missing
    /// (§8.1), presence-over-budget (§8.3), storage-on-state (§4.2 S4),
    /// archive-unaligned (§4.4), state-stamp-foreign (S1–S2, with --deep),
    /// shm-memlock-low (§7.4), admin-unreachable and router-version-skew. The
    /// deployment is read through a session in its namespace; the routers'
    /// admin space and the presence domain through one in no namespace. A
    /// check whose input could not be had is unobservable, with the reason —
    /// never clean — and so is every check that reads presence when no zk2
    /// token is visible. Exit 0 every check asked is clean, 1 a finding at or
    /// above --fail-on (default warning), 2 no verdict: a check left
    /// unobservable, an empty scope, or a run that could not start.
    Doctor(DoctorArgs),
    /// Explain why a key is silent, one established fact at a time.
    ///
    /// The non-verdict, itemised (#214): a rung ladder over facts the engine
    /// already holds: scope reach, grammar, registry declaration, the
    /// liveliness roster, declared publishers, storage coverage, a stored
    /// value, freshness, admin reachability. Every rung answers established /
    /// not-established (with its reason) / NOT ASKED — "not asked" is never
    /// rendered as "no" (RFC 09 §5.1 O4), because silence is never a verdict
    /// (RFC 05 §3.1). "No publisher declared" never reads as a bug: publishers
    /// declare lazily, on the first publication (RFC 08 §6.1). The default run
    /// costs the control plane only; `--for` adds the one data-plane rung. Exit
    /// 0 = nothing found and everything checked looks healthy; 1 = a cause was
    /// established — the finding; 2 = the observation was impaired.
    Why(WhyArgs),
    /// Watch conditions on the bus and print each state change as ndjson.
    ///
    /// Emits TRANSITIONS (#227). A foreground observer — explicitly launched, one process per
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
    /// Wait for an expectation on the bus, exit-coded for CI.
    ///
    /// An await (#160). The subscriber is declared BEFORE the window opens (not-asked is not
    /// "no", RFC 09 §5.1 O4). Exit 0 = met; 1 = not met on a clean
    /// observation; 2 = the observation cannot carry the claim (drops under
    /// a completeness claim, session failure). `--absent` is legitimate
    /// ONLY because of that 2.
    Expect(CheckExpectArgs),
    /// Cutover acceptance: the old key family is silent and the new one speaks.
    ///
    /// Half one of RFC 09 §6: assert a retired key family SILENT while the
    /// new plane carries traffic. Three verdicts, three exits: 0 = old silent AND new speaking; 1 =
    /// the old family still speaks; 2 = everything was quiet — a
    /// non-verdict, because a dead fleet passes the silence half for free.
    /// The leak check's meaning is stated, not inferred: anything outside
    /// `<base>/v1/` that is not the old root.
    Cutover(CheckCutoverArgs),
    /// Which deprecated subjects are actually gone from the bus.
    ///
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
    /// Cutover acceptance: probe one origin with concrete keys, as a consumer.
    ///
    /// Half two of RFC 09 §6: a consumer-shaped, CONCRETE-KEY probe. "A probe
    /// MUST build its keys the way the product builds them" — a `*`-origin
    /// probe cannot catch a broken origin path. An origin id is called
    /// directly; a hostname resolves through the RFC 06 §6 identity bridge
    /// first, and the probe FAILS if the bridge yields nothing. Fanning out is
    /// what `blob locate` does; this verb refuses to.
    Probe(CheckProbeArgs),
    /// Run a producer's registry as a conformance suite against the bus.
    ///
    /// #222: 0 = conforms, 1 = an assertion is not met, 2 = unproven. One
    /// assertion per declared surface, three states each — met, not
    /// met, unknowable with its reason (RFC 13 §3) — and unknowable is never
    /// folded into not met. Every origin the roster shows running the
    /// producer is CALLED: introspect and each concrete `read` procedure,
    /// with no arguments (`error/invalid-args` is a reply, and met). A write
    /// is never called; it is met when the origin's served slice declares
    /// it. `error/unsupported` or `error/gated` from a `when` procedure is
    /// exempt and says so — unless the device's registration document
    /// claims the capability (RFC 04 §5) — and from any other procedure it
    /// is not met (RFC 08 §6.1). Silence from a rostered origin is not met:
    /// alive ⇒ callable (RFC 13 §2). Then v1's registry checks, scoped to the
    /// producer: slice sync, describe totality, schema drift; with `--for`,
    /// each declared subject — a window proves presence, never absence, so a
    /// subject that did not speak is unknowable. With `--registry` the suite
    /// is those files (the contract the build ships); without, what the
    /// fleet serves.
    Conform(CheckConformArgs),
    /// Validate one payload against its schema, exit-coded for CI.
    ///
    /// No bus write (#159): 0 = valid, 1 = does not conform, 2 = could not
    /// check. The schema comes from a live `describe` (give --producer) or an
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
    /// Time a zk2 operation's replies, per replier key.
    ///
    /// Latency is measured **per reply**, where it arrives, on this tool's
    /// clock (a round trip, never a stamp), and attributed by the key it
    /// went on (spec §5.1 O3), so a fast replier in a fan-out is not charged
    /// the slowest one's round trip. The calls go out as `call`'s do: one
    /// address `BestMatching`, a `*` or an unbound parameter a fan-out
    /// (`All`), only to an operation declaring `fanout = "allowed"` (O2).
    /// Envelopes are a population of their own, unattributed; a call that
    /// drew nothing is silence, counted and never averaged in (O5); each
    /// token holder of the selection is tallied against the calls it sent no
    /// value in. Only an operation declared `idempotent` benches by default:
    /// a benchmark repeats, and repeating a write is a different act from
    /// measuring it. Exit 0 values only, 1 an envelope among them, 2 no
    /// value at all.
    Call(BenchCallArgs),
}

#[derive(Subcommand)]
pub(crate) enum SchemaCmd {
    /// One revision's schema artifacts, and the type each member names.
    ///
    /// `<iface>` alone is the one revision `--contracts` holds or the
    /// deployment's descriptors name; `@<fingerprint>` (or a prefix of one)
    /// picks one among several. Without `--full` each artifact is listed by
    /// id, kind and name; with it, or with a resource named, its document
    /// follows — a JSON Schema as carried, a protobuf descriptor set read as
    /// its messages and enums. Answered from `--contracts` with no session
    /// when it holds the revision. Exit 2 when the revision cannot be had.
    Show(SchemaShowArgs),
}

#[derive(Subcommand)]
pub(crate) enum IfaceCmd {
    /// Every interface provided or required, and by whom.
    ///
    /// From one presence read and every instance's descriptor: providers by
    /// interface token or, for the tokenless set, by descriptor; consumers
    /// by the roles descriptors declare; the revisions providers name. A read
    /// that ran to its timeout is possibly incomplete, and says so.
    List(IfaceListArgs),
    /// One interface: providers, consumers, and each revision's contract.
    ///
    /// Each provider with its token and its descriptor's fingerprint, the
    /// resources it exposes by the compact rule (spec §3.3) when its
    /// revision is in hand, its unavailable list and cardinality bounds; each
    /// role bound to the interface; and each revision's contract —
    /// resources, kinds, QoS, fan-out, types — from `--contracts` or
    /// retrieved from its holders (spec §8.4). `@<fingerprint>` narrows the
    /// view to one revision, and shows its contract even when no provider
    /// names it now. Exit 2 when nothing provides, requires or describes the
    /// interface: silence is not an answer.
    Show(IfaceShowArgs),
}

#[derive(Subcommand)]
pub(crate) enum NamespaceCmd {
    /// The namespaces zk2 services hold instance tokens in.
    ///
    /// The command to run *before* you have a namespace: one liveliness read
    /// of `**/zk2/*/*/@zk/instance/*` on a session in no namespace, every
    /// token attributed to the prefix before its `zk2/`. The bus-root
    /// deployment (no namespace) is listed as `(empty)` and selected with
    /// `--namespace ''`.
    List(NamespaceListArgs),
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
    /// The mesh as the admin space answers it: nodes, edges, mentions.
    ///
    /// #118: nodes, edges, and who only got mentioned. Their pictures are
    /// unlabeled circles; ours says which of admin space and liveliness backs
    /// each element. Nodes whose admin space is off render "heard of, not
    /// queryable" — never omitted.
    Graph(AdminGraphArgs),
}

#[derive(Subcommand)]
pub(crate) enum AclCmd {
    /// Generate the router's `access_control` block from an enrollment
    ///
    /// zk2's three grant shapes (spec §11.1), compiled per principal from
    /// the enrollment and the contracts (`--contracts`):
    ///
    ///   Own      put, delete, serve and declare tokens under the service's
    ///            prefix, each verbatim subtree spelled out (`**` never
    ///            crosses one): */@stream/**, */@state/**, */@op/**, @zk/**;
    ///            and the @adv subtrees where its contracts declare history
    ///   Consume  subscribe or GET on what its bindings name (R1, R2), the
    ///            @adv subtrees where it reads with history, and liveliness
    ///            reads on each provider's @zk/** (0.8)
    ///   Call     query on the specific …/@op/<op> keys, and the same
    ///            liveliness reads on each service called
    ///
    /// Egress is checked by inclusion against the query's or subscription's
    /// own key (§11.2): every consumer or caller selector over what a
    /// provider serves joins its egress grant, and its ingress reply. Contract
    /// bundles are open. Under --default-permission allow, zenoh evaluates no
    /// allow rule, so each grant compiles into denies of its complement,
    /// enumerated from the contracts. Every rule names its grant and the fact
    /// it exists for. A principal the plan cannot place is refused, never
    /// dropped: exit 1.
    ///
    /// The enrollment file (examples/zk2/acl/ holds two):
    ///
    ///   namespace = "fleet-a"                  # optional; default --namespace
    ///   [[principal]]
    ///   user     = "thruster-l"                # a usrpwd user, or cn = "…"; never a zid
    ///   services = ["vehicle-01/thruster-l"]   # and archives = […], tools = […]
    ///   [[service]]
    ///   address    = "vehicle-01/thruster-l"
    ///   implements = ["thruster.v1"]
    ///   [service.bindings.cmd]                 # a role; interface and resources
    ///   providers = ["vehicle-01/teleop"]      #   from the contract's [requires]
    ///   [[service]]
    ///   address = "vehicle-01/executor"
    ///   [service.bindings.plan]
    ///   interface = "mission_plan.v1"          # a role of its own manifest
    ///   providers = ["ground/fleet-mgr"]
    ///   params    = { vehicle = "self.system" }
    ///   [[service.calls]]
    ///   interface  = "nav.v2"
    ///   providers  = ["vehicle-01/navigation"]
    ///   operations = ["set_origin"]            # default: every operation
    ///   [[archive]]
    ///   address = "vehicle-01/archive"
    ///   records = ["zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01"]
    ///   peers   = ["ground/archive"]
    ///   [[tool]]
    ///   name = "ops"                           # bindings and calls, no address
    // Verbatim, so the table and the enrollment example keep their lines:
    // clap would otherwise fold them into one.
    #[command(verbatim_doc_comment)]
    Gen(AclGenArgs),
}

#[derive(Subcommand)]
pub(crate) enum StorageCmd {
    /// List configured storages and which declared state they cover.
    ///
    /// Each declared state family is judged covered, partial or uncovered
    /// (RFC 04 §4, issue #14).
    List(StorageListArgs),
    /// Generate the router's storage config from the registry and a deployment file
    ///
    /// Plan the router's storages from the registry and a small deployment
    /// file, and emit the `plugins.storage_manager` block with
    /// `garbage_collection.lifespan` DERIVED (RFC 09 §2, #393).
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
    /// Which producers declare which blob tiers (registry only, no bus traffic).
    ///
    /// A declaration of an `@blob` tier is a capability, never possession.
    List(BlobListArgs),
    /// Ask every origin who holds a blob — a tiny reply, never the bytes.
    ///
    /// RFC 07 §2.5, total across tiers since v1.17: `have`/`manifest` for
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
    /// Fetch a blob from one origin, verified before it reaches disk.
    ///
    /// From **one** origin's concrete key, at data-low, verifying every reply
    /// against the content root before it reaches disk (RFC 07 §2.1).
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
        /// Endpoint to listen on, repeatable — a context that listens opens
        /// a peer session, where every other context opens a client.
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
pub(crate) enum ServiceCmd {
    /// Every running service: instances, interfaces, descriptors.
    ///
    /// One liveliness read of `zk2/*/*/@zk/**` in the deployment's namespace
    /// (spec §8.1), then every instance's descriptor (§3.3). Each interface
    /// row keeps its two sources apart — the token's fingerprint prefix
    /// beside the descriptor's full fingerprint — and an interface in the
    /// tokenless set (U22) is known from the descriptor alone. A pure
    /// consumer is listed too: every instance holds an instance token. A read
    /// that ran to its timeout is possibly incomplete, and says so: a service
    /// missing from it may still be up.
    List(ServiceListArgs),
    /// One service: each instance's tokens and the descriptor it serves.
    ///
    /// The descriptor as served (spec §3.3): its interfaces with their full
    /// fingerprints, the tokenless set, capabilities, unavailable resources
    /// and cardinality bounds, and every role with its bindings. Exit 2 when
    /// presence shows no instance: silence is not an answer.
    Show(ServiceShowArgs),
}

/// How a session reaches the bus, and nothing about which deployment it
/// reads: the connection half of every zk2 verb (#612, FJ4).
///
/// `namespace list` takes it alone, because it looks *across* namespaces;
/// every resolved verb takes it inside `NamespaceArgs`. The ladders are the
/// v1 flags' (flag > env > active context > default), and the help is theirs.
#[derive(Args, Clone)]
pub(crate) struct SessionArgs {
    /// Use a named context from the config file for this invocation
    /// (default: the file's `current` pointer; env `ZENCTL_CONTEXT`).
    #[arg(long, value_name = "NAME", add = ArgValueCandidates::new(completion::contexts))]
    pub(crate) context: Option<String>,
    /// Endpoint to connect to, repeatable (e.g. `tcp/127.0.0.1:7447`).
    ///
    /// The session is a zenoh client of these endpoints: no listener of its
    /// own, no gossip, nothing the mesh can route through. An endpoint
    /// nothing answers fails the command (exit 2) rather than reading as an
    /// empty bus, and one that does not parse is refused by name (exit 2).
    #[arg(long, short = 'c')]
    pub(crate) connect: Vec<String>,
    /// Endpoint to listen on, repeatable — which makes the session a peer.
    #[arg(long, short = 'l')]
    pub(crate) listen: Vec<String>,
    /// Enable multicast scouting (off by default, and think first).
    #[arg(long)]
    pub(crate) scouting: bool,
    /// Seconds to wait for replies (default 5; a context may override the
    /// default). Bounds the presence read, each descriptor GET and each
    /// contract retrieval, a state GET, and a call — which, given none,
    /// waits what its operation recommends (`timeout_ms`).
    #[arg(long, value_name = "SECS")]
    pub(crate) timeout: Option<u64>,
    /// Zenoh JSON5 config file: the passthrough that reaches a secured bus.
    ///
    /// Loaded as the base layer; --connect/--listen/--scouting apply on top
    /// when given. A file that sets a session namespace is refused: the
    /// namespace is `--namespace`'s to set.
    #[arg(long, value_name = "FILE", env = "ZENCTL_ZENOH_CONFIG")]
    pub(crate) zenoh_config: Option<PathBuf>,
    #[command(flatten)]
    pub(crate) out: OutputArgs,
}

/// A zk2 resolved verb's bus: the deployment's namespace, and the
/// connection (#612, FJ4).
#[derive(Args, Clone)]
pub(crate) struct NamespaceArgs {
    /// The deployment namespace: the session is opened in it.
    ///
    /// zk2 keys are base-relative (`zk2/<system>/<service>/…`), and a
    /// deployment's services run in a zenoh session namespace that prefixes
    /// them on the wire. A resolved verb opens its session in the same one,
    /// so it reads what a consumer of the deployment reads. `--base` is the
    /// same flag, and so is the active context's `base`. Resolution: flag >
    /// env > active context > empty — the bus-root deployment, no namespace.
    /// `zenctl namespace list` finds the namespaces in use.
    #[arg(long, visible_alias = "base", value_name = "NS", env = "ZENCTL_BASE",
          add = ArgValueCandidates::new(completion::namespaces))]
    pub(crate) namespace: Option<String>,
    #[command(flatten)]
    pub(crate) session: SessionArgs,
}

/// Contracts known without the bus (#612, FJ4).
#[derive(Args, Clone)]
pub(crate) struct ContractArgs {
    /// Contracts known offline, repeatable: an authoring file (`*.toml`), a
    /// directory of them, or a `.history` root.
    ///
    /// They seed the contract store, so a revision held here is never
    /// retrieved from the bus (spec §8.5) — and a question they answer alone
    /// opens no session at all. A file that is not a valid contract is
    /// reported on stderr; a path that loads nothing is refused (exit 2).
    #[arg(long = "contracts", value_name = "PATH")]
    pub(crate) contracts: Vec<PathBuf>,
}

/// `service list`'s flags.
#[derive(clap::Args)]
pub(crate) struct ServiceListArgs {
    /// Only this system's services (`zk2/<system>/*`).
    #[arg(long, value_name = "SYSTEM")]
    pub(crate) system: Option<String>,
    #[command(flatten)]
    pub(crate) ns: NamespaceArgs,
}

/// `service show`'s flags.
#[derive(clap::Args)]
pub(crate) struct ServiceShowArgs {
    /// The service address, `<system>/<service>`.
    #[arg(value_name = "SYSTEM/SERVICE", value_parser = addr_arg,
          add = ArgValueCandidates::new(completion::services))]
    pub(crate) address: zenkey_model::grammar::Addr,
    #[command(flatten)]
    pub(crate) ns: NamespaceArgs,
}

/// `iface list`'s flags.
#[derive(clap::Args)]
pub(crate) struct IfaceListArgs {
    #[command(flatten)]
    pub(crate) ns: NamespaceArgs,
}

/// `iface show`'s flags.
#[derive(clap::Args)]
pub(crate) struct IfaceShowArgs {
    /// `<name>.v<major>`, optionally `@<fingerprint>` (or a prefix of one).
    #[arg(value_name = "IFACE[@FP]", value_parser = revision_arg,
          add = ArgValueCandidates::new(completion::ifaces))]
    pub(crate) target: RevisionSpec,
    #[command(flatten)]
    pub(crate) contracts: ContractArgs,
    #[command(flatten)]
    pub(crate) ns: NamespaceArgs,
}

/// `schema show`'s flags.
#[derive(clap::Args)]
pub(crate) struct SchemaShowArgs {
    /// `<name>.v<major>`, optionally `@<fingerprint>` (or a prefix of one).
    #[arg(value_name = "IFACE[@FP]", value_parser = revision_arg,
          add = ArgValueCandidates::new(completion::ifaces))]
    pub(crate) target: RevisionSpec,
    /// Only this resource: `<kind token>/<template>`
    /// (`stream/bandwidth/{ns}/{iface}`), or its template alone when only
    /// one resource has it. Implies the full documents.
    pub(crate) resource: Option<String>,
    /// Print every artifact's document, not just its id, kind and name.
    #[arg(long)]
    pub(crate) full: bool,
    #[command(flatten)]
    pub(crate) contracts: ContractArgs,
    #[command(flatten)]
    pub(crate) ns: NamespaceArgs,
}

/// `namespace list`'s flags: a connection, and no namespace.
#[derive(clap::Args)]
pub(crate) struct NamespaceListArgs {
    #[command(flatten)]
    pub(crate) session: SessionArgs,
}

/// `graph`'s flags.
#[derive(clap::Args)]
pub(crate) struct GraphArgs {
    /// Emit Graphviz instead of the table (pipe to `dot -Tsvg`).
    ///
    /// A foreign schema, so `--format` has no say over it: passing both is
    /// a usage error, not a silent preference.
    // #243, and see `refuse_foreign_format` for why not `conflicts_with`.
    #[arg(long)]
    pub(crate) dot: bool,
    #[command(flatten)]
    pub(crate) ns: NamespaceArgs,
}

/// `compat`'s flags.
#[derive(clap::Args)]
pub(crate) struct CompatArgs {
    /// The earlier revision: `*.toml`, `*.bundle.json`, or `<iface>[@<fp>]`.
    pub(crate) old: String,
    /// The candidate revision, in any of the same spellings.
    pub(crate) new: String,
    #[command(flatten)]
    pub(crate) contracts: ContractArgs,
    #[command(flatten)]
    pub(crate) ns: NamespaceArgs,
}

/// `<name>.v<major>[@<fingerprint>]`, as `iface show`, `schema show` and
/// `compat` take it: an interface, and optionally which revision of it —
/// the full fingerprint (`sha256:` optional) or a prefix of its hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RevisionSpec {
    pub(crate) iface: zenkey_model::grammar::IfaceId,
    /// Lowercase hex, 1 to 64 digits, without `sha256:`.
    pub(crate) fingerprint: Option<String>,
}

impl std::fmt::Display for RevisionSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.fingerprint {
            Some(fp) => write!(f, "{}@{fp}", self.iface),
            None => write!(f, "{}", self.iface),
        }
    }
}

/// Parse a `RevisionSpec` — clap's refusal, exit 2, naming the argument.
pub(crate) fn revision_arg(s: &str) -> Result<RevisionSpec, String> {
    let (iface, fp) = match s.split_once('@') {
        Some((i, f)) => (i, Some(f)),
        None => (s, None),
    };
    let iface = iface
        .parse::<zenkey_model::grammar::IfaceId>()
        .map_err(|e| format!("{e} (e.g. `tc.netif.v1`)"))?;
    let fingerprint = match fp {
        None => None,
        Some(f) => {
            let hex = f.strip_prefix("sha256:").unwrap_or(f);
            if hex.is_empty()
                || hex.len() > 64
                || !hex
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                return Err(format!(
                    "{f:?} is not a fingerprint: 1 to 64 lowercase hex digits, \
                     `sha256:` optional"
                ));
            }
            Some(hex.to_owned())
        }
    };
    Ok(RevisionSpec { iface, fingerprint })
}

/// Parse a service address, `<system>/<service>`.
fn addr_arg(s: &str) -> Result<zenkey_model::grammar::Addr, String> {
    s.parse().map_err(|e| format!("{e}"))
}

/// Options shared by every v1 command: the deployment base, the registry
/// source, and the connection. zk2's verbs take `NamespaceArgs` (or
/// `SessionArgs` alone) instead, until FJ9 retires this struct.
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
    /// keys start at `v1/`. `zenctl namespace list` finds the namespaces zk2
    /// services use.
    #[arg(long, env = "ZENCTL_BASE")]
    pub(crate) base: Option<String>,
    /// Use a named context from the config file for this invocation
    /// (default: the file's `current` pointer; env `ZENCTL_CONTEXT`).
    #[arg(long, value_name = "NAME", add = ArgValueCandidates::new(completion::contexts))]
    pub(crate) context: Option<String>,
    /// Local registry directory (`registry/*.{toml,kdl}`), repeatable. Joined with
    /// the live bus as a union (RFC 08 §6.1): a producer's served slice wins,
    /// these files fill the gaps, and a disagreement is reported — never
    /// silently overwritten. With the bus unreachable they answer alone.
    #[arg(long, value_name = "DIR")]
    pub(crate) registry: Vec<PathBuf>,
    /// Endpoint to connect to, repeatable (e.g. `tcp/127.0.0.1:7447`).
    ///
    /// The session is a zenoh client of these endpoints: no listener of its
    /// own, no gossip, nothing the mesh can route through (RFC 09 §5). An
    /// endpoint nothing answers fails the command (exit 2) rather than
    /// reading as an empty bus, and one that does not parse — no `tcp/`,
    /// say — is refused by name (exit 2).
    #[arg(long, short = 'c')]
    pub(crate) connect: Vec<String>,
    /// Endpoint to listen on, repeatable.
    ///
    /// Listening makes the session a zenoh peer — reachable, and part of the
    /// mesh's gossip — which an explorer otherwise never is (RFC 09 §5).
    #[arg(long, short = 'l')]
    pub(crate) listen: Vec<String>,
    /// Enable multicast scouting.
    ///
    /// OFF by default — with a --zenoh-config too, unless the file itself
    /// states `scouting.multicast.enabled` — and you should think before
    /// turning it on: a scouting explorer joins whatever mesh it can find,
    /// which is how a throwaway session ends up talking to a production fleet.
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
    ///
    /// Only what the file states counts: one that does not name `mode` gets
    /// the explorer's client session (a peer when it listens), and one that
    /// does not name `scouting.multicast.enabled` gets multicast off — zenoh's
    /// own defaults (peer, multicast on) do not leak in.
    #[arg(long, value_name = "FILE", env = "ZENCTL_ZENOH_CONFIG")]
    pub(crate) zenoh_config: Option<PathBuf>,
    #[command(flatten)]
    pub(crate) out: OutputArgs,
}

/// `--format` selects among **zenkey's own three renderings** of a report. A
/// foreign document format — `--dot` (`admin graph`, `graph`),
/// `--json5`, `export --prom` — is somebody else's schema, so the two are
/// mutually exclusive (#243).
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

    // Walk to the leaf: `admin graph` and `storage gen` are both two deep,
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
    for (id, flag) in [("dot", "--dot"), ("json5", "--json5"), ("prom", "--prom")] {
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
/// `echo`, `watch`, `serve`, `watchdog` and `doctor --transitions` emit rows as they
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
        ["echo"] | ["serve"] | ["watchdog"] | ["watch"] => true,
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

/// The `get` verb's flags — one struct the dispatcher hands over whole,
/// destructured in the verb rather than in `run()` (#354). The raw form's,
/// with `get state` beside it as a subcommand (#612, FJ5): the
/// `snapshot diff` shape, so the two forms never share a positional.
#[derive(clap::Args)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub(crate) struct GetArgs {
    /// Any key expression, params included (`key?k=v`): a WIRE selector, on
    /// a session in no namespace. For a zk2 state resource through its
    /// contract, use `get state`.
    #[arg(required = true, add = ArgValueCandidates::new(completion::keys))]
    pub(crate) selector: Option<String>,
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
    #[command(subcommand)]
    pub(crate) cmd: Option<GetSub>,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

#[derive(Subcommand)]
pub(crate) enum GetSub {
    /// Read one zk2 state resource of one owner, through its contract.
    ///
    /// RESOLVED, unlike `get <SELECTOR>`: the state resource of
    /// `<IFACE>`'s contract (from the bus, or `--contracts`) on the owner
    /// `<ADDRESS>`, in the deployment's namespace. A GET on the owner's keys
    /// for it — target All, consolidation Latest, the owner alone answering
    /// (spec §4.2 S4) — one member when every template parameter is given,
    /// every member otherwise. Each value is rendered through the contract
    /// (§7.2), a deletion within the owner's window is a deletion, and each
    /// stamp names its clock. Silence is not "no value": exit 2 (S6).
    ///
    /// `--last-known <ARCHIVE>` asks an `archive.v1` service instead
    /// (S5): the value it recorded, its stamp, its type identity and whether
    /// alignment confirmed it — LAST-KNOWN, never current, and every
    /// rendering says so. Exit codes: 0 an answer, 2 silence or a refused
    /// input.
    State(Box<StateGetArgs>),
}

/// A template parameter, `NAME=VALUE`, as `call`, `get state` and `watch`
/// take it (`--param`).
pub(crate) fn param_arg(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((name, value)) if !name.is_empty() => Ok((name.to_owned(), value.to_owned())),
        _ => {
            Err("expected NAME=VALUE, the template parameter's name and its unslugged value".into())
        }
    }
}

/// `get state`'s flags.
#[derive(clap::Args)]
pub(crate) struct StateGetArgs {
    /// The owner, `<system>/<service>`: one service, since the owner alone
    /// answers for its state (S4).
    #[arg(value_name = "SYSTEM/SERVICE", value_parser = addr_arg,
          add = ArgValueCandidates::new(completion::services))]
    pub(crate) address: zenkey_model::grammar::Addr,
    /// `<name>.v<major>`, optionally `@<fingerprint>` (or a prefix of one).
    #[arg(value_name = "IFACE[@FP]", value_parser = revision_arg,
          add = ArgValueCandidates::new(completion::ifaces))]
    pub(crate) target: RevisionSpec,
    /// The state resource: its template (`interfaces/{ns}/{iface}`), or
    /// `state/<template>` / `@state/<template>`.
    pub(crate) resource: String,
    /// A template parameter, `NAME=VALUE` with the value unslugged,
    /// repeatable. A parameter not given is a wildcard: every member.
    #[arg(long = "param", value_name = "NAME=VALUE", value_parser = param_arg)]
    pub(crate) params: Vec<(String, String)>,
    /// Read LAST-KNOWN state from this archive (`<system>/<service>`, an
    /// `archive.v1` service) instead of asking the owner (S5). Never
    /// current, and labelled so in every format; every parameter must be
    /// given, since an archive is read one key at a time.
    #[arg(long, value_name = "ARCHIVE", value_parser = addr_arg)]
    pub(crate) last_known: Option<zenkey_model::grammar::Addr>,
    #[command(flatten)]
    pub(crate) contracts: ContractArgs,
    #[command(flatten)]
    pub(crate) ns: NamespaceArgs,
}

/// `call`'s flags.
#[derive(clap::Args)]
pub(crate) struct CallArgs {
    /// The service, `<system>/<service>`. A `*` in either position fans
    /// the call out over every service it selects.
    #[arg(value_name = "SYSTEM/SERVICE", add = ArgValueCandidates::new(completion::services))]
    pub(crate) address: String,
    /// `<name>.v<major>`, optionally `@<fingerprint>` (or a prefix of one).
    #[arg(value_name = "IFACE[@FP]", value_parser = revision_arg,
          add = ArgValueCandidates::new(completion::ifaces))]
    pub(crate) target: RevisionSpec,
    /// The operation: its template (`interfaces/{ns}/{iface}/set`,
    /// `diagnostics`), or `@op/<template>`.
    pub(crate) operation: String,
    /// The request: inline JSON, `@file`, or `-` for stdin — the bytes as
    /// given for a raw request type. Omitted: `{}`, or no bytes for a raw
    /// type. Encoded as the contract says, never checked against its JSON
    /// Schema: the owner refuses what does not decode (`invalid_request`).
    #[arg(value_name = "JSON|@FILE|-")]
    pub(crate) request: Option<Source>,
    /// A template parameter, `NAME=VALUE` with the value unslugged,
    /// repeatable; a rest parameter takes one per chunk. A parameter not
    /// given is a wildcard, which makes the call a fan-out.
    #[arg(long = "param", value_name = "NAME=VALUE", value_parser = param_arg)]
    pub(crate) params: Vec<(String, String)>,
    /// Retries after silence (O4). Honoured only for an idempotent
    /// operation: any other is called once whatever this says, and a
    /// fan-out is never retried.
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub(crate) retries: u32,
    #[command(flatten)]
    pub(crate) contracts: ContractArgs,
    #[command(flatten)]
    pub(crate) ns: NamespaceArgs,
}

/// `watch`'s flags.
#[derive(clap::Args)]
pub(crate) struct WatchArgs {
    /// The service, `<system>/<service>`, either position `*` (`*/tc`).
    #[arg(value_name = "SYSTEM/SERVICE", add = ArgValueCandidates::new(completion::services))]
    pub(crate) address: String,
    /// `<name>.v<major>`, optionally `@<fingerprint>` (or a prefix of one).
    #[arg(value_name = "IFACE[@FP]", value_parser = revision_arg,
          add = ArgValueCandidates::new(completion::ifaces))]
    pub(crate) target: RevisionSpec,
    /// The stream, state or event resource: its template
    /// (`bandwidth/{ns}/{iface}`), or `<kind token>/<template>`.
    pub(crate) resource: String,
    /// A template parameter, `NAME=VALUE` with the value unslugged,
    /// repeatable. A parameter not given is a wildcard.
    #[arg(long = "param", value_name = "NAME=VALUE", value_parser = param_arg)]
    pub(crate) params: Vec<(String, String)>,
    /// Stop after this many seconds (default: until interrupted).
    #[arg(long = "for", value_name = "SECS")]
    pub(crate) for_secs: Option<f64>,
    /// Stop after this many samples (default: until interrupted).
    #[arg(long, value_name = "N")]
    pub(crate) count: Option<u64>,
    #[command(flatten)]
    pub(crate) contracts: ContractArgs,
    #[command(flatten)]
    pub(crate) ns: NamespaceArgs,
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
    /// Full wire selectors to capture, repeatable — this session is in no
    /// namespace, so a selector is the key as the wire carries it
    /// (`prod/zk2/**`). Default: the deployment's zk2 data, `<ns>/zk2/**`,
    /// whose `**` reaches no verbatim chunk (`@stream`, `@state`, …): name
    /// one to capture it (`prod/zk2/*/*/*/@stream/**`).
    #[arg(value_name = "SELECTOR", add = ArgValueCandidates::new(completion::keys))]
    pub(crate) selectors: Vec<String>,
    /// Output file. An existing file is refused (exit 2) unless
    /// --overwrite: a capture is the one artifact this verb exists to keep.
    #[arg(long, short = 'o', value_name = "FILE")]
    pub(crate) out: String,
    /// Replace an existing --out file instead of refusing it.
    #[arg(long)]
    pub(crate) overwrite: bool,
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
    /// before it. Repeatable; the zk2 conditions of the watchdog's
    /// vocabulary — `rate-above <SEL> <HZ>`, `rate-below <SEL> <HZ>`,
    /// `silent-for <SEL> <SECS>`, `dropped`, `doctor <CHECK-ID>` (zk2's
    /// doctor, run in the namespace). The rules that judge v1's registry,
    /// roster or alert plane are refused (exit 2). The file carries a state
    /// preamble (the owners' own answer, spec §4.2 S4), the pre-roll, the
    /// trigger record where it fired, then `--post` seconds more.
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
    pub(crate) ns: NamespaceArgs,
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

/// The `export` verb's flags (#228) — one struct, the `GenArgs` pattern.
#[derive(clap::Args)]
pub(crate) struct ExportArgs {
    #[command(flatten)]
    pub(crate) selector: SelectorArgs,
    /// Address to serve `/metrics` on (`--listen` is the zenoh transport's).
    /// Loopback by default; a non-loopback address exposes the bus's shape
    /// to the network and needs --i-know.
    #[arg(long, value_name = "ADDR", default_value = "127.0.0.1:9184")]
    pub(crate) bind: String,
    /// Bind a non-loopback --bind address. The refusal you are overriding
    /// names its reason.
    #[arg(long = "i-know")]
    pub(crate) i_know: bool,
    /// Validate payloads against their served schemas (RFC 08 §7), budgeted
    /// per key per second so the exporter never becomes a load test; the
    /// `valid`/`invalid` populations move only with this. Without it every
    /// sample is `not_validated`, and the surface says so.
    #[arg(long)]
    pub(crate) validate: bool,
    /// Run zk2's doctor every SECS, in the deployment's namespace (`--base`),
    /// and expose its findings as `zenkey_doctor_finding{check_id,severity}`.
    /// Off by default: a doctor run costs the control plane (RFC 13 §3,
    /// frugality). Without it `zenkey_doctor_info{state="not_asked"}` is the
    /// honest series.
    #[arg(long, value_name = "SECS")]
    pub(crate) doctor_every: Option<f64>,
    /// Bound on distinct series; overflow is counted under
    /// `zenkey_series_suppressed_total{reason="max_series"}`.
    #[arg(long, value_name = "N", default_value_t = 10_000)]
    pub(crate) max_series: usize,
    /// Observe for --for seconds, fold once, print the snapshot as a report
    /// (`--format`) and exit — no listener. For a script that wants one
    /// scrape's worth of the surface as JSON.
    #[arg(long)]
    pub(crate) once: bool,
    /// With --once: how long to observe before the one fold, seconds.
    #[arg(
        long = "for",
        value_name = "SECS",
        default_value_t = 5.0,
        requires = "once"
    )]
    pub(crate) for_secs: f64,
    /// With --once: print the Prometheus exposition text instead of a
    /// report — a foreign schema, so not with --format.
    #[arg(long, requires = "once")]
    pub(crate) prom: bool,
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
    /// silence would read as an empty fleet (RFC 05 §3.1). An existing file
    /// is refused too (exit 2), before any session, unless --overwrite.
    #[arg(long, short = 'o', value_name = "FILE", required = true)]
    pub(crate) out: Option<String>,
    /// Replace an existing --out file instead of refusing it.
    #[arg(long)]
    pub(crate) overwrite: bool,
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
    /// Compare two .zsnap files, offline.
    ///
    /// No bus. Both spans are stated, the facets (value, verdict, registration,
    /// holder) stay apart, and an origin an alignment could not pair is listed,
    /// never dropped. Exit 0 identical, 1 they differ, 2 a file could not be
    /// read.
    Diff(SnapshotDiffArgs),
}

#[derive(clap::Args)]
pub(crate) struct SnapshotDiffArgs {
    /// The earlier snapshot.
    pub(crate) a: String,
    /// The later snapshot.
    pub(crate) b: String,
    /// Align origins across deployments — by the `source` label their
    /// health/sensor documents carry, then by producer set — and roll the
    /// diff up per subject. Refuses (exit 2) over any origin it cannot
    /// pair, and lists them.
    #[arg(long)]
    pub(crate) normalize_origins: bool,
    /// An explicit origin pairing, `A=B` (a's origin = b's), repeatable;
    /// decided before any automatic pairing. Requires --normalize-origins.
    #[arg(long = "map", value_name = "A=B", requires = "normalize_origins")]
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
    /// header's — or both are empty, which cannot tell two deployments
    /// apart.
    #[arg(long)]
    pub(crate) force_base: bool,
    /// Write what is not this replay's to write: recorded deletes that
    /// fall off the state class (RFC 04 §1.2, v1.12), and a zk2 service's
    /// own keys where that service runs (P3, spec §6).
    #[arg(long = "i-know")]
    pub(crate) i_know: bool,
    /// Publish into this deployment namespace, through a session opened in
    /// it: every key is moved from the capture's base into NS, so a
    /// replayer stands in for the owners it recorded in a namespace of its
    /// own (spike S13). Typed only, never read from the environment or a
    /// context. A zk2 service's own key replayed into the namespace it was
    /// recorded in — NS equal to the capture's base, or no --namespace at
    /// all — is refused unless --i-know (P3).
    #[arg(long, value_name = "NS")]
    pub(crate) namespace: Option<String>,
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

/// The `serve` verb's flags (#612, FJ8a) — one struct the dispatcher hands
/// over whole, destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct ServeArgs {
    /// The address the mock owner runs at, `<system>/<service>`: yours to
    /// name. One whose instance token is already present is refused
    /// unless --i-know.
    #[arg(value_name = "SYSTEM/SERVICE", value_parser = addr_arg,
          add = ArgValueCandidates::new(completion::services))]
    pub(crate) address: zenkey_model::grammar::Addr,
    /// `<name>.v<major>`, optionally `@<fingerprint>` (or a prefix of one).
    #[arg(value_name = "IFACE[@FP]", value_parser = revision_arg,
          add = ArgValueCandidates::new(completion::ifaces))]
    pub(crate) target: RevisionSpec,
    /// The operation: its template (`diagnostics`), or `@op/<template>`.
    pub(crate) operation: String,
    /// The fixed reply: inline JSON, `@file`, or `-` for stdin (read once)
    /// — the bytes as given for a raw response type — encoded as the
    /// contract's response type, as `call` encodes a request. Omitted: a
    /// synthesized response (--seed). Not with --refuse.
    #[arg(value_name = "JSON|@FILE|-", conflicts_with = "refuse")]
    pub(crate) reply: Option<Source>,
    /// Refuse every call with this envelope code instead of replying
    /// (spec §5.2).
    #[arg(long, value_enum, value_name = "CODE")]
    pub(crate) refuse: Option<RefuseCode>,
    /// The refusal's message, for a human (never parsed).
    #[arg(long, value_name = "TEXT", requires = "refuse")]
    pub(crate) message: Option<String>,
    /// The refusal's cause: required by `unavailable`, refused with any
    /// other code (spec §5.2).
    #[arg(long, value_enum, value_name = "CAUSE", requires = "refuse")]
    pub(crate) cause: Option<CauseArg>,
    /// A role binding, `ROLE=SYSTEM/SERVICE[,…]`, repeatable (R1): a
    /// contract's required role must be bound, or a service does not
    /// start.
    #[arg(long = "bind", value_name = "ROLE=PROVIDERS")]
    pub(crate) binds: Vec<String>,
    /// The seed a synthesized reply is made with.
    #[arg(long, default_value_t = 42)]
    pub(crate) seed: u64,
    /// Exit after N calls (default: until interrupted).
    #[arg(long, value_name = "N")]
    pub(crate) count: Option<u64>,
    /// Stop after this many seconds (default: until interrupted).
    #[arg(long = "for", value_name = "SECS")]
    pub(crate) for_secs: Option<f64>,
    /// Start beside an instance already running at the address: a second
    /// answerer of its operations (P3) and a split-brain on every exclusive
    /// resource (spec §6).
    #[arg(long = "i-know")]
    pub(crate) i_know: bool,
    #[command(flatten)]
    pub(crate) contracts: ContractArgs,
    #[command(flatten)]
    pub(crate) ns: NamespaceArgs,
}

/// The envelope codes a mock owner may refuse a call with (spec §5.2).
/// `fanout_forbidden` is the runtime's own (O2), never a handler's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum RefuseCode {
    #[value(name = "invalid_request")]
    InvalidRequest,
    #[value(name = "not_found")]
    NotFound,
    Unavailable,
    Forbidden,
    Busy,
    Internal,
    App,
}

/// An `unavailable` envelope's cause (spec §2.3, §5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum CauseArg {
    Build,
    Config,
    Capability,
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
    /// `doctor <CHECK-ID>`, `origin-down <ORIGIN>`, `dropped`,
    /// `alert-firing <SEL> [<MIN-SEVERITY>]` (info | warning | critical,
    /// default warning). Selectors are full wire form (this session is
    /// un-namespaced, RFC 09 §5); a doctor rule runs zk2's doctor once per
    /// tick, its named checks only, through a second session opened in the
    /// deployment's namespace (`--base`); an alert-firing rule asks the
    /// alert plane once per tick.
    #[arg(long = "rule", value_name = "RULE", required = true)]
    pub(crate) rules: Vec<String>,
    /// Seconds between evaluations — the one period flag (#307).
    #[arg(long, value_name = "SECS", default_value_t = 5.0)]
    pub(crate) every: f64,
    /// Stop after N evaluations (default: run until interrupted). A bounded
    /// run exits on how its rules ended: 1 if any is firing, else 2 if any
    /// is unobservable, else 0.
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

/// The `check conform` verb's flags — one struct the dispatcher hands over
/// whole, destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct CheckConformArgs {
    /// The producer whose registry slice is the suite.
    #[arg(long, value_parser = chunk_arg,
          add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: String,
    /// Call this origin only (`h-…` or `@service`), not every origin the
    /// roster shows. Off the roster, its silence is unknowable, not a
    /// failure.
    #[arg(long, value_name = "ORIGIN")]
    pub(crate) origin: Option<String>,
    /// Listen passively for this many seconds and judge each declared
    /// subject: presence, QoS, payload, kind, rate, cardinality. No window =
    /// the subject assertions are not asked.
    #[arg(long = "for", value_name = "SECS")]
    pub(crate) for_secs: Option<f64>,
    /// Also judge freshness against `ttl_s` and the declared `[budget]` —
    /// a state snapshot and a health fetch, real query load.
    #[arg(long)]
    pub(crate) deep: bool,
    /// Write the assertions as JUnit XML: not met is a failure, unknowable
    /// is SKIPPED (never failed), an exemption rides system-out.
    #[arg(long, value_name = "PATH")]
    pub(crate) junit: Option<std::path::PathBuf>,
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
    #[arg(value_parser = chunk_arg, add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: String,
    /// Procedure path, e.g. `introspect`.
    #[arg(value_parser = procedure_arg, add = ArgValueCandidates::new(completion::procedures))]
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
    #[arg(long, value_parser = chunk_arg,
          add = ArgValueCandidates::new(completion::producers))]
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

/// The `bench call` verb's flags (#612, FJ8a) — one struct the dispatcher
/// hands over whole, destructured in the verb rather than in `run()` (#354).
#[derive(clap::Args)]
pub(crate) struct BenchCallArgs {
    /// The service, `<system>/<service>`. A `*` in either position fans
    /// each call out over every service it selects.
    #[arg(value_name = "SYSTEM/SERVICE", add = ArgValueCandidates::new(completion::services))]
    pub(crate) address: String,
    /// `<name>.v<major>`, optionally `@<fingerprint>` (or a prefix of one).
    #[arg(value_name = "IFACE[@FP]", value_parser = revision_arg,
          add = ArgValueCandidates::new(completion::ifaces))]
    pub(crate) target: RevisionSpec,
    /// The operation: its template, or `@op/<template>`.
    pub(crate) operation: String,
    /// The request, as `call` takes it: inline JSON, `@file`, or `-` for
    /// stdin. Omitted: `{}`, or no bytes for a raw type.
    #[arg(value_name = "JSON|@FILE|-")]
    pub(crate) request: Option<Source>,
    /// A template parameter, `NAME=VALUE` with the value unslugged,
    /// repeatable. A parameter not given is a wildcard: a fan-out.
    #[arg(long = "param", value_name = "NAME=VALUE", value_parser = param_arg)]
    pub(crate) params: Vec<(String, String)>,
    /// Calls to issue (default 100).
    //
    // `--calls`, not `--count` (#307): `--count` is a stop bound on a
    // stream everywhere else, and this is the size of the experiment.
    #[arg(long, value_name = "N")]
    pub(crate) calls: Option<usize>,
    /// Calls in flight at once (1 = strictly sequential).
    #[arg(long, default_value_t = 1)]
    pub(crate) concurrency: usize,
    /// Bench an operation not declared `idempotent`: each call is a write,
    /// repeated. A fan-out the operation forbids stays refused (O2).
    #[arg(long = "i-know")]
    pub(crate) i_know: bool,
    #[command(flatten)]
    pub(crate) contracts: ContractArgs,
    #[command(flatten)]
    pub(crate) ns: NamespaceArgs,
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
    /// The enrollment file (TOML): principals, the services, archives and
    /// tools they run, their bindings and calls. See `zenctl acl gen --help`
    /// for the shape.
    #[arg(long, value_name = "FILE")]
    pub(crate) enrollment: PathBuf,
    #[command(flatten)]
    pub(crate) contracts: ContractArgs,
    /// The router's default_permission (spec §11.2): `deny`, where grants are
    /// allow rules (RECOMMENDED), or `allow`, where zenoh evaluates no allow
    /// rule and each grant compiles into denies of its complement.
    #[arg(long, visible_alias = "posture", value_enum, value_name = "PERMISSION",
          default_value_t = DefaultPermission::Deny)]
    pub(crate) default_permission: DefaultPermission,
    /// Emit the router config fragment on stdout: the `access_control` block,
    /// a comment per rule naming its grant and its fact, and for a south
    /// region the `gateway` block. A foreign schema, zenoh's, so `--format`
    /// has no say over it.
    // #243, and see `refuse_foreign_format` for why not `conflicts_with`.
    #[arg(long, conflicts_with_all = ["check", "explain"])]
    pub(crate) json5: bool,
    /// Compare the plan against a router config file (`--against`): missing,
    /// extra and changed rules, subjects and policies, an identity the
    /// enrollment does not know, a south region not placed. Exit 0
    /// identical / 1 findings / 2 not asked.
    ///
    /// A file, not the bus: access control is enforced per hop and the
    /// running configuration is not observable (spec §11.3).
    #[arg(long, requires = "against", conflicts_with = "explain")]
    pub(crate) check: bool,
    /// With --check: the router's JSON5 config file, read through zenoh's
    /// own loader so what is compared is what zenohd would run.
    #[arg(long, value_name = "FILE", requires = "check")]
    pub(crate) against: Option<PathBuf>,
    /// Does PRINCIPAL (a subject id, user or CN) hold MESSAGE on KEY (the
    /// wire key, namespace included), via which rules, in which direction?
    /// Inclusion by zenoh-keyexpr. Exit 0.
    #[arg(long, num_args = 3, value_names = ["PRINCIPAL", "KEY", "MESSAGE"])]
    pub(crate) explain: Option<Vec<String>>,
    /// Guard a constrained face too (spec §8.5): the far principal's policy
    /// carries the `@zk` and `@stream` denies, and loses its presence and
    /// contract grants (bindings resolve statically there, R7).
    #[arg(long, value_name = "constrained", requires_all = ["attach", "far"])]
    pub(crate) face: Option<Face>,
    /// With --face: how the far side attaches. `client`: one far-side or
    /// gateway session, a client of this router. `south-region`: a far
    /// router in a south region of this one (`gateway.south`; needs
    /// --region). `router` is refused: a deny on a router-to-router link
    /// hides the declarations but lets their key strings cross.
    #[arg(long, value_enum, value_name = "ATTACHMENT", requires = "face")]
    pub(crate) attach: Option<Attach>,
    /// With --face: the far side's principal (its id, user or CN): the
    /// gateway session, or the far router.
    #[arg(long, value_name = "PRINCIPAL", requires = "face")]
    pub(crate) far: Option<String>,
    /// With --attach south-region: the far router's `region_name`, which
    /// this router's `gateway.south` lists.
    #[arg(long, value_name = "NAME", requires = "face")]
    pub(crate) region: Option<String>,
    #[command(flatten)]
    pub(crate) ns: NamespaceArgs,
}

/// `--default-permission`: the router's posture (spec §11.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum DefaultPermission {
    Deny,
    Allow,
}

/// The one face profile spec §8.5 describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Face {
    /// A bandwidth-limited link: radio, cell, tactical, satellite.
    Constrained,
}

/// `--attach`: how the far side of a constrained face attaches (§8.5, U23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Attach {
    /// One far-side session, or a gateway session, as a client.
    Client,
    /// A far router in a south region of this router.
    SouthRegion,
    /// A far router linked router to router: refused, with why.
    Router,
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
    #[arg(long, value_parser = chunk_arg,
          add = ArgValueCandidates::new(completion::producers))]
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

#[derive(Subcommand)]
pub(crate) enum ConfigCmd {
    /// Read a resource's running configuration and any pending change.
    ///
    /// The served schema beside every running value, its source, and any
    /// pending change (on-bus).
    Get(ConfigGetArgs),
    /// Change one group of a resource, typed against the served schema
    /// (on-bus).
    ///
    /// The read-back is fetched first, so a value is read as the kind the
    /// producer declares and refused here — in the producer's own words —
    /// when the producer would refuse it. A `reach` group needs `--confirm`
    /// — or `--token`, joining a pending change that has a window — and a
    /// yes; a `contract` group is refused with the restart named.
    Set(ConfigSetArgs),
    /// Make a pending change permanent (on-bus).
    Confirm(ConfigTokenArgs),
    /// Undo a pending change now (on-bus).
    Cancel(ConfigTokenArgs),
    /// Move a pending change's deadline (on-bus).
    Extend(ConfigExtendArgs),
    /// Write a change into the producer's persisted configuration.
    ///
    /// Its own key, so an ACL grants it apart from the change (on-bus).
    Persist(ConfigPersistArgs),
}

/// The `config get` verb's flags.
#[derive(clap::Args)]
pub(crate) struct ConfigGetArgs {
    /// Origin to target: a host id (`h-3fa9c2d41b7e`) or `*` for the fleet.
    pub(crate) origin: String,
    /// Producer name.
    #[arg(value_parser = chunk_arg, add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: String,
    /// The resource — the chunk an ACL grants by: a device, an interface.
    #[arg(value_parser = chunk_arg)]
    pub(crate) resource: String,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `config set` verb's flags.
#[derive(clap::Args)]
pub(crate) struct ConfigSetArgs {
    /// Origin to target: one host id — a change never fans out.
    pub(crate) origin: String,
    /// Producer name.
    #[arg(value_parser = chunk_arg, add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: String,
    /// The resource the group belongs to.
    #[arg(value_parser = chunk_arg)]
    pub(crate) resource: String,
    /// The group to change — the unit that has a class and that the write
    /// key names.
    #[arg(value_parser = chunk_arg)]
    pub(crate) group: String,
    /// The values, `name=value`, one or more; a subset of the group changes
    /// those alone.
    #[arg(value_name = "NAME=VALUE", required = true)]
    pub(crate) values: Vec<String>,
    /// Validate and report what would change, without touching the device.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Arm a rollback: the change is undone after SECS unless confirmed.
    /// Required for a `reach` group, unless --token joins a change that has
    /// one.
    #[arg(long, value_name = "SECS")]
    pub(crate) confirm: Option<u64>,
    /// Join the pending change named by TOKEN (RFC 05 §5.1, v1.50): this
    /// group is applied under that change's window and confirmed, cancelled
    /// or rolled back with it — so it takes no --confirm of its own.
    #[arg(long, value_name = "TOKEN", conflicts_with = "confirm")]
    pub(crate) token: Option<String>,
    /// Refuse the change if the document's revision has moved past this.
    #[arg(long, value_name = "N")]
    pub(crate) expect_revision: Option<u64>,
    /// A key a retry carries, so a lost reply is not a doubled write.
    #[arg(long, value_name = "KEY")]
    pub(crate) idempotency_key: Option<String>,
    /// Who is asking, for the change event's record — a claimed label,
    /// never an authentication (RFC 06 §5.5).
    #[arg(long, value_name = "NAME")]
    pub(crate) actor: Option<String>,
    /// A request id for the change event's record, likewise claimed.
    #[arg(long, value_name = "ID")]
    pub(crate) request_id: Option<String>,
    /// Send a `reach` change without being asked (for a script that has
    /// decided) — or a --confirm or --token change to a group whose class
    /// no read-back established, which may be one.
    #[arg(long)]
    pub(crate) yes: bool,
    /// Skip the read-back: values ride by their spelling and the producer
    /// judges the rest. With --confirm, the group may be reach, so --yes
    /// (or a terminal's yes) is asked for.
    #[arg(long)]
    pub(crate) no_validate: bool,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `config confirm|cancel|persist` verbs' flags: a change named by its
/// token.
#[derive(clap::Args)]
pub(crate) struct ConfigTokenArgs {
    /// Origin to target: one host id.
    pub(crate) origin: String,
    /// Producer name.
    #[arg(value_parser = chunk_arg, add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: String,
    /// The resource the change is on.
    #[arg(value_parser = chunk_arg)]
    pub(crate) resource: String,
    /// The change's token, as `set` answered it.
    pub(crate) token: String,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `config persist` verb's flags: the change by its token, or — with
/// none — the read-back's `last_change` (RFC 05 §5.1, v1.50).
#[derive(clap::Args)]
pub(crate) struct ConfigPersistArgs {
    /// Origin to target: one host id.
    pub(crate) origin: String,
    /// Producer name.
    #[arg(value_parser = chunk_arg, add = ArgValueCandidates::new(completion::producers))]
    pub(crate) producer: String,
    /// The resource the change is on.
    #[arg(value_parser = chunk_arg)]
    pub(crate) resource: String,
    /// The change's token: the pending change's, or `last_change`'s. Omitted,
    /// the read-back's `last_change` is persisted — the way a change made
    /// without a window survives a restart.
    pub(crate) token: Option<String>,
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

/// The `config extend` verb's flags.
#[derive(clap::Args)]
pub(crate) struct ConfigExtendArgs {
    #[command(flatten)]
    pub(crate) change: ConfigTokenArgs,
    /// The new rollback window, seconds from now.
    #[arg(long, value_name = "SECS")]
    pub(crate) by: u64,
}

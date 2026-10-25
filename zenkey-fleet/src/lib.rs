//! Fleet engine for zk2 tooling (issue #15; zk2 since #612).
//!
//! The core of `zenctl`, `zengui` and `zenwatch`: everything a bus
//! explorer needs that is not presentation, in five layers — see **The
//! map** below. It reads a zk2 deployment through the zk2 runtime
//! (`zenkey` 0.20, by path) and its session-free model (`zenkey-model`):
//! presence from instance and interface tokens, descriptors, contract
//! bundles retrieved by fingerprint, and every payload rendered through
//! the contract that declares it.
//!
//! **Two sessions** (decided 2026-10-08). A resolved verb reads through a
//! session opened **in** the deployment's namespace, as the deployment's
//! own consumers do, and the functions it calls spell base-relative
//! `zk2/…` keys. A raw verb — a wire selector, the admin space — runs on a
//! session in **no** namespace, and sees the wire as it really is, full
//! keys included: that is what lets it spot a key outside the deployment.
//! [`bus::lens`] and [`model::lens`] are the bridge: a raw observer's key
//! resolved through the namespace, presence and the contracts in hand. The
//! RFC 05 §2.1 fan-in discipline of a raw GET lives in exactly one place
//! ([`bus::query::fleet_get`] — target `All`, consolidation `None`,
//! attribution by the reply's own key), and every liveliness GET goes
//! through [`bus::presence::liveliness_read`].
//!
//! **A watch session** (#830, core §8.1, 0.37). A liveliness GET from a
//! session that holds a liveliness subscriber, history or not, can complete
//! without a token that is live (zenoh 1.10.1 and main), so a reader that
//! follows presence live holds its liveliness subscribers — a
//! [`Monitor`]'s, an alarms or telemetry reader's — on a session of its own
//! ([`DoctorBus::watch`]), and makes every read on the others. Every
//! liveliness subscriber here is declared through
//! `zenkey::presence::declare_subscriber`, which marks its session, and a
//! read on a marked session is refused before anything is asked
//! ([`Error::NotAsked`]); so is one through a session cut off from every
//! other zenoh node ([`bus::presence::cut_off`]).
//!
//! **v1 left at FJ9** (#612). The registry slice sets, RFC 08 §6
//! introspection, the v1 roster, the schema-aware decode seam, the v1
//! judges (`why`, `cutover`, `retired`, `conform` and its registry checks)
//! and the profile-backed features (v1's configuration, blobs, the exporter,
//! alerts, kinds and budgets) live on the `v1` branch, which releases
//! 0.14.x. Every frontend on `main` runs on this crate: zenctl since FJ9,
//! zenwatch since FL3 and zengui since FL11b (#614), and none links the v1
//! engine (`zenkey-fleet =0.18.0`) any more.
//!
//! # The map
//!
//! Five strata, and the arrow between them only ever points one way. A
//! sample enters at the top and leaves at the bottom as something a frontend
//! can draw:
//!
//! ```text
//!   bus/     holds a session      →  observations
//!   model/   holds values         →  meaning
//!   judge/   holds meaning        →  verdicts
//!   report/  the serialized shapes every layer above hands out
//!   tape/    traffic as a thing: captured, replayed, manufactured, timed
//! ```
//!
//! * **[`bus`]** — everything whose job needs a live session. `session`,
//!   `query`, `monitor`, `write`, `serve`, `admin`, `scout`, `seed` (a late
//!   joiner's: subscribe, then S4's GET, merged by stamp, core §4.2 0.26,
//!   by `zenkey_model::seed` since 0.27),
//!   and zk2's `presence` (its `PresenceFeed` keeps `model::presence_live`
//!   current from a monitor, #614), `contracts`, `operation`, `consume`,
//!   `lens` (its `LensFeed` follows presence moves and re-reads when
//!   nudged, #614 FL4), `why`
//!   (the reads a silence is explained from, #702), `conform` (a
//!   service's suite, #703), `stall` (what a window's reader sees of its
//!   own receive path: its transports and a ticker, core §2.7 0.27),
//!   `health` (a `health.v1` reading, #721), `config` (a `config.v1`
//!   reading, and one act through the runtime's client, #768) and `alarms`
//!   (an `alarms.v1` reader: subscribed, seeded by S4's GET, its own
//!   receive path watched, live or one-shot, #800) and `telemetry` (a
//!   `telemetry.v1` continuity reader: the metric resources the
//!   descriptors declare, their members and the owners' tokens in one
//!   queue, presence seeded, state seeded by S4's GET, #819), and
//!   `desired` (a `desired.v1` tool's reading of each target's roles: the
//!   targets' and authors' presence, the contracts, the target's reports
//!   and S4's GET of each slice, made again within a window, #840). The
//!   RFC 05 §2.1 fan-in discipline lives here exactly once, in
//!   [`bus::query::fleet_get`], and everything in the layer that asks a raw
//!   question goes through it. Every liveliness GET goes through
//!   [`bus::presence::liveliness_read`] in the same way, on the unbounded
//!   handler spec §8.1 requires (zenoh#2678), and never on a watch session
//!   or through a session cut off (core §8.1, 0.37). This layer returns
//!   observations and never a verdict about one.
//!
//!   Its event sources are **`Stream`s** (#343), not only `recv` loops:
//!   [`EventStream::into_stream`], [`SeededSubscriber`] (a direct impl),
//!   and a `stream()` on [`ScoutStream`], [`MockResponder`] and
//!   [`MatchingEvents`]. The last three borrow rather than consume, because
//!   the `&self` receiver is what lets a query be answered while its stream
//!   is held and a scout be stopped after one; a blanket `impl Stream` would
//!   have taken `&mut self` and spent that. The `recv`/`next` methods stay —
//!   a loop is still the clearer shape for a drain that also selects on
//!   something else.
//!
//!   [`watchdog`] is the one that is not a `Stream` but a
//!   [`Straw`] (#397): it yields transitions *and* returns a
//!   [`WatchdogSummary`], with the acknowledged monitor teardown between the
//!   two. That is a shape `Stream` has no room for, and the only reason this
//!   crate speaks a second streaming vocabulary at all.
//!
//! * **[`model`]** — everything that can do its job from values already in
//!   hand. zk2's `catalog`, `render`, `target`, `compat`, `namespace` and
//!   `lens` (a raw observer's key resolved rung by rung, its payload
//!   checked, its stamp attributed), `presence_live` (each address up, down
//!   or unobservable, from a seed and the liveliness events after it, with
//!   every move a value, #614; down only on a complete read, never on a
//!   withdrawal heard live, core §8.1, 0.37), `presence_watch` (one
//!   watcher's reading of
//!   it, held or gone per address a pattern matches: `instance-gone`'s in
//!   zenctl and zenwatch alike, #747), `skeleton` (the declared keyspace a
//!   tree starts from, merged with what was observed, #614 FL4), `identity`
//!   (each key's resolution, kept and bounded), `config` (a `config.v1`
//!   reading's freshness and projection, a change typed, pre-checked and
//!   redacted before it is sent, #768), `alarms` (one reader's book of a
//!   scope's alarms: each key merged by stamp and read as §2.6 says, each
//!   owner's seed and window, the owners in scope, and the transitions a
//!   notifier routes, #800), `telemetry` (one reader's book of metric
//!   members: each sample read as §2.6 says, each pair of consecutive
//!   conforming ones judged with the presence the reader saw between them,
//!   in one order, a state member's in the late joiner's merge order,
//!   #819), `desired` (a role's slice resolved from the author's
//!   declaration and the target's binding, its reading projected into
//!   `zenkey_model::desired`'s, and how long a target was behind, #840),
//!   `timeline`, `snapshot` and
//!   `snapshot_diff` over it, `acl` and `storage` (a router's config from
//!   an enrollment or a deployment file), the `structural` ladder bytes no
//!   schema reaches fall to, `stats`, `tree` and `diff`, plus the two
//!   mechanisms every long-running projection shares (`bounded`,
//!   `examples`). Nothing here takes a session, and that is load-bearing:
//!   it is what lets a frontend replay a `.zrec` through the same
//!   projections it runs live.
//!
//! * **[`judge`]** — everything that takes a position. `doctor` and its
//!   `doctor_delta`, `expect`, `probe`, `condition`, `field`, `why` (a
//!   silence explained rung by rung, #702), `conform` (a service against
//!   the revision it claims, #703), `health` (`health.v1`'s verdicts,
//!   #721), `config` (`config.v1`'s §5 answers, #768), `alarms`
//!   (`alarms.v1`'s §5 answers, #800), `telemetry` (`telemetry.v1`'s,
//!   #819) and `desired` (`desired.v1`'s, each the model's, #840), and
//!   [`judge::common`] for the vocabulary they share. The honesty rules (the
//!   tooling guide, which carries RFC 13 over to zk2) bite hardest here, so
//!   the layer states them once.
//!
//! * **[`report`]** — every serde-pinned wire shape in the crate, split by
//!   domain. Its module doc carries the placement rule, which is the answer
//!   to "where does this struct go?" whenever the struct has a `Serialize`
//!   on it.
//!
//! * **[`tape`]** — traffic as a thing rather than an event. `record`,
//!   `ingest`, `snapshot`, `trigger`, `mock`, `generate`, `synth`, `bench`.
//!   It sits beside the others rather than under them because it both reads
//!   from the bus and writes back to it.
//!
//! **Placing a new module.** Ask, in order: does it need a session
//! (`bus/`), can it answer from values in hand (`model/`), does it say
//! whether something is *wrong* (`judge/`), does it turn a stream into a
//! recording or back (`tape/`)? A new serde-pinned struct is not a module
//! question at all — it goes to [`report`], by the rule stated there.
//!
//! What is deliberately **not** here: configuration. Where the operator
//! keeps their connection contexts is `zenkey-explorer-config`'s job; this
//! crate has no stratum for `~/.config`, and forcing `dirs` and `toml` on a
//! library consumer so two binaries could read a TOML file was the tell.

// docs.rs builds on nightly with `--cfg docsrs` (see Cargo.toml). Inert
// everywhere else — a stable `cargo doc` never sets the cfg (#325). The
// crate has no feature axes since FJ9 (#612): the v1 decode seam and its
// codec features left with the v1 dependency, and zk2's decode is
// `zenkey_model::decode`, unconditional.
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod bus;
pub mod error;
pub mod judge;
pub mod model;
pub mod report;
pub mod tape;
// ─── the supported surface ──────────────────────────────────────────────────
//
// **The rule: the crate root is the whole supported surface.** Every type and
// function a frontend is meant to use is re-exported here, and a path through
// a module (`zenkey_fleet::model::lens::Lens`) is a spelling of the same
// item, never the only way to reach one. The modules stay `pub` because
// their docs are where the reasoning lives and because a reader browsing by
// module should not hit a wall — but nothing supported is *only* there.
//
// Why it matters: both frontends had drifted into a mix of the two, and
// which spelling a call site used said nothing about how supported the item
// was. With the rule, "is this ours to use?" is answered by looking at this
// block, and adding a public item without adding it here is the omission
// that stands out.
//
// What is deliberately *not* here: `report`'s row and cell types, which are
// the rendering vocabulary rather than the engine's — a frontend reaches
// those through `zenkey_fleet::report::*`, and only the reports the verbs
// below actually **return** are lifted to the root.

pub use judge::condition::{
    AlarmRead, CondWindow, Condition, DoctorWatch, Eval, RuleSet, RuleState, SweepOutcome,
    WatchdogSpec, WindowExamples, watchdog,
};
pub use judge::expect::{ExpectAim, ExpectSpec, run_expect};
pub use judge::field::{
    DeclaredPaths, FieldObservation, FieldSpec, KeyFieldContext, KeyFields, PathStats, run_field,
};
pub use judge::probe::run_probe;
/// The traits [`watchdog`] is driven through (#397), re-exported so a
/// consumer needs them in scope without taking a direct dependency on
/// `sipper` — and so the version this engine speaks is the one it hands out.
pub use sipper::{Sender, Sipper, Straw};
// The mock owner (#612, FJ8a): `gen` and `serve` bring up a real zk2
// service at the operator's address, and synthesize what it publishes and
// answers from the contract.
pub use tape::generate::{GenPattern, GenSpec, MemberArg, build_plan as gen_plan, run_gen};
pub use tape::mock::{
    AddressPresence, MockAnswer, ServeSpec, Served, address_presence, check_address,
    marker as synthetic_marker, serve as serve_operation,
};
pub use tape::synth::{Synth, Synthesized, member_type, size_class};
pub use tape::trigger::{TriggerEvent, TriggerSpec, record_on, state_projection};

pub use bus::admin::{
    AdminEntry, RouterVerification, admin_doc_omits_loopback, admin_get, admin_get_within,
    declared_entities, declared_entities_within, declared_entity_selectors, mesh_links, render_dot,
    routers, storages, topology, topology_with_instances,
};
pub use bus::monitor::{
    EventStream, FleetEvent, Monitor, MonitorCore, MonitorSpec, SampleSource, SampleView,
    StampProvenance, StreamItem, WatchId,
};
pub use bus::query::{
    Answer, DEFAULT_MAX_REPLIES, FleetAnswer, GetOpts, RepeatingQuery, declare_repeating,
    declare_repeating_any, fleet_get,
};
pub use bus::scout::{ScoutStream, scout};
pub use bus::seed::{SeedItem, SeedPolicy, SeededSubscriber, seed_subscribe};
pub use bus::serve::{MockResponder, ServedQuery, declare_responder};
pub use bus::session::{
    OPEN_TIMEOUT, OpenFailure, check_namespace, open, open_in_namespace, open_owner_in_namespace,
    open_reporting, open_reporting_within, open_with_config,
};
pub use bus::write::{
    MatchingEvents, Publication, WireQos, WriteAct, check_concrete, check_retire,
    declare_publication,
};
pub use judge::common::EXPANSION_CAP;
// zk2's doctor (#612, FJ6): the run, its two halves, and the delta a
// notifier compares runs with.
pub use judge::doctor::{
    AdminSpace, ArchiveKeys, DEFAULT_GRACE, DEFAULT_PRESENCE_BUDGET, DOMAIN_SELECTORS, DoctorBus,
    DoctorObservation, DoctorSpec, DomainTokens, Memlock, StateStamps, judge as judge_doctor,
    observe as observe_doctor, run_doctor,
};
pub use judge::doctor_delta::doctor_delta;
// zk2's `why` (#702): a key's or a service's silence, rung by rung — the
// reads, the session-free judge, and the two in a row.
pub use bus::why::{
    ArchiveRead, KeyAnswer, KeyReply, WhyObservation, WhySpec, WhyTarget, observe as observe_why,
    run_why,
};
pub use judge::why::judge as judge_why;
// zk2's `check conform` (#703): one service against the revision it claims,
// as a suite — the reads, the session-free judge, and the two in a row.
pub use bus::conform::{
    ConformObservation, ConformSpec, FanoutSeen, Heard, OpObserved,
    SAMPLE_CAP as CONFORM_SAMPLE_CAP, observe as observe_conform, run_conform,
};
pub use judge::conform::judge as judge_conform;
// `health.v1` (#721, PF): every service's health, or one's, read as the
// profile's reader reads it — the reads, the session-free judge, and the
// two in a row.
pub use bus::config::{
    ConfigCall, ConfigGet, ConfigObservation, ConfigOp, ConfigSpec, ConfigTarget, ConfigWindow,
    call as call_config, get as get_config, observe as observe_config, run_config,
};
pub use bus::health::{
    AcrossFace, HealthObservation, HealthOpening, HealthSpec, HealthTarget,
    observe as observe_health, run_health,
};
pub use judge::config::judge as judge_config;
pub use judge::health::judge as judge_health;
// `alarms.v1` (#800, AC): a scope's alarms read as the profile's reader
// reads them — the live reader a watchdog rule and a notifier hold, the
// one-shot reading, the session-free book both fold into and its
// transitions, and the judge.
pub use bus::alarms::{
    AlarmFeed, AlarmObservation, AlarmRefresh, AlarmSpec, PresenceSeen as AlarmPresenceSeen,
    ReadLog as AlarmReadLog, Refreshed as AlarmRefreshed, get as get_alarms,
    observe as observe_alarms, run_alarms,
};
pub use judge::alarms::judge as judge_alarms;
pub use model::alarms::{
    AlarmBook, AlarmSample, AlarmScope, ArchivedAlarm, BookSpec as AlarmBookSpec, KeyReading,
    SeedEnd as AlarmSeedEnd, SeedRead as AlarmSeedRead,
};
// `desired.v1` (#840, DD): each target's roles against their authors'
// documents, read as the profile's tool reads them — the reads, the
// session-free projection into the model's reading, and the judge.
pub use bus::desired::{DesiredSpec, observe as observe_desired, run_desired};
pub use judge::desired::judge as judge_desired;
pub use model::desired::{
    Behind as DesiredBehindTracker, DesiredRead, FaceWord as DesiredFaceWord,
};
// `redundancy.v1` (#841, RC): who serves each governed interface, read
// as the profile's tool reads it — two presence reads a grace apart, the
// session-free projection into the model's reading, and the judge.
pub use bus::redundancy::{
    DEFAULT_GRACE as REDUNDANCY_DEFAULT_GRACE, RedundancyObservation, RedundancySpec,
    observe as observe_redundancy, run_redundancy,
};
pub use judge::redundancy::{Standby, judge as judge_redundancy, recognised_standby};
// `telemetry.v1` (#819, TC): a scope's metric members read as the
// profile's reader reads them — the reads, the session-free book they fold
// into, and the judge.
pub use bus::telemetry::{
    DEFAULT_SETTLE as TELEMETRY_DEFAULT_SETTLE, Opening as TelemetryOpening,
    RoundTrips as TelemetryRoundTrips, SeedSeen as TelemetrySeedSeen, TelemetryObservation,
    TelemetrySpec, metric_resources, observe as observe_telemetry, run_telemetry,
};
pub use judge::telemetry::judge as judge_telemetry;
pub use model::config::{Assignment, Given, TypedValues};
pub use model::telemetry::{
    MemberBook, MetricResource, MetricSample, MetricType, PairPresence, PairSeen, PairTally,
    SampleVia as MetricVia, TelemetryBook, TelemetryScope,
};
// Types reachable *through* root-exported ones (#350).
pub use model::bounded::DEFAULT_MAX_KEYS;
pub use model::tree::{TreeNode, TreeRow, TreeRows};
// The structural ladder needs no codec.
pub use model::structural::{OBSERVE_LIMIT, structural, structural_value};
// zk2 (#612, FJ3): presence and its §8.1 liveliness chokepoint, contract
// retrieval, the catalog and its offline contracts, and honest rendering.
// The generic verbs keep a zk2 name at the root — `observe` and `render`
// alone would read as the crate's.
pub use bus::contracts::{BundleStore, DEFAULT_MAX_CONTRACTS, UNAVAILABLE_TTL};
pub use bus::presence::{
    CUT_OFF_DURING, DESCRIBE_CONCURRENCY, NAMESPACE_SELECTOR, Scope as PresenceScope, cut_off,
    describe as describe_instances, liveliness_read, namespace_listing,
    observe as observe_presence, read_tokens, refuse_beside, service_listing,
};
// Live presence, per address (#614, FL1): the session-free projection, and
// the feed that seeds it and keeps it current from a monitor.
pub use bus::presence::{
    PresenceFeed, RESEED_BACKOFF, RESEED_BACKOFF_MAX, SeedOutcome, WITHDRAWAL_SETTLE,
    presence_selectors,
};
pub use model::catalog::{
    Catalog, ContractSet, ContractState, Contracts, DescriptorRead, LoadProblem,
    Observed as ObservedPresence, Revision, namespaces, type_view,
};
pub use model::compat::compat;
pub use model::presence_live::{
    DEFAULT_MAX_GONE, LiveAddress, LiveInstance, LivePresence, PresenceBasis, PresenceIgnored,
};
// `instance-gone`'s per-address reading of it (#747): one implementation
// for zenctl's watchdog and zenwatch's rule.
pub use model::presence_watch::{AddressMove, AddressWatch, covering_scope};
// Its moves are wire shapes since zenwatch's notifications carry them (#614,
// FL2): computed by `model::presence_live`, pinned in `report`, spelled here
// as before.
pub use model::render::{
    Member, render as render_payload, render_detail, render_resource,
    render_with as render_payload_with, resolved_revision,
};
pub use report::{AddressStatus, MemberId, PresenceChange, PresenceSource, PresenceTransition};
// A deployment namespace on the wire (#612, FJ9): the two spellings a raw
// observer moves between.
pub use model::namespace::{join as with_namespace, strip as strip_namespace};
// The raw observers' lens (#612, FJ8b): a wire key resolved through the
// namespace, presence and the contracts in hand, rung by rung (the tooling
// guide's O2), and the read that keeps it current.
pub use bus::lens::{LensFeed, REFRESH_MIN as LENS_REFRESH_MIN, read as read_lens};
pub use model::lens::{
    Checked, Lens, Resolution, Resolved, check_payload, conformance, declared_qos, observed_qos,
    qos_mismatch, sample_conformance,
};
// A tree's model (#614, FL4): the declared keyspace from the lens, merged
// with the observed snapshot and the watch set; and each key's identity,
// resolved once. `merge` keeps a name of its own at the root, as `diff`
// does: alone it would read as the crate's.
pub use model::identity::IdentityCache;
pub use model::skeleton::{
    DeclBasis, DeclRef, Evidence, MergedNode, NodeStats, NodeStatus, Skeleton, SkeletonChunk,
    SkeletonCoverage, SkeletonNode, merge as merge_skeleton,
};
// zk2's acts and reads through a contract (#612, FJ5): planned and refused
// without a session (`model::target`), then made through the runtime's
// `Client`, `Fleet` and `Consumer` (`bus::operation`, `bus::consume`).
pub use bus::consume::{
    StateRead, WATCH_BUFFER, Watch, get_state, last_known as last_known_state, stamp,
    watch as watch_resource,
};
pub use bus::operation::{OperationCall, call as call_operation};
pub use model::target::{
    CallPlan, OwnedKey, Target as ResolvedTarget, check_values, encode_request, encode_response,
    owned_key, plan_call, resource as resolve_resource,
};
pub use tape::record::{rfc3339_from_unix, rfc3339_now};
// The judging vocabulary a caller can drive directly (#349's evidence
// structs among them).
pub use judge::condition::{
    SilenceEvidence, TickEvidence, judge_alert_firing, judge_doctor_check, judge_instance_gone,
    presence_observer,
};
// The remaining items a frontend actually calls. Every one of these was
// reachable only by module path (#350) — which said nothing about whether it
// was ours to use.
pub use bus::teardown::DECLARE_TIMEOUT;
pub use error::{BoxedCause, Error, NotAsked, Result, one_line};
pub use judge::field::DEFAULT_MAX_PATHS;
// zk2's access control (spec §11, #612 FJ7): planned from an enrollment
// and the contracts, checked against a router's config file, explained.
pub use model::acl::{
    AclOptions, check_acl, check_face, explain_acl, link_says, plan_acl, to_json5 as acl_plan_json5,
};
// `diff` is `value_diff` at the root: a bare `diff` beside `byte_diff` reads
// as *the* diff.
pub use model::diff::{ByteDiff, Change, ValueDiff, byte_diff, diff as value_diff};
pub use model::retain::{RetentionBudget, RetentionStats};
pub use model::snapshot::{fold_latest, holder_of, row_of as snapshot_row, stamper_of};
pub use model::snapshot_diff::{DiffOpts, diff_snapshots};
pub use model::stats::{KeyStats, StampClass, StatsTable};
pub use model::storage::{
    StorageInputs, check_storages, check_storages_against, explain as explain_storage,
    on_owner_state, plan as plan_storages_from, plan_storages, to_json5 as storage_plan_json5,
};
pub use model::timeline::{
    ArrivalAxis, ArrivalOrdering, Break, HlcAxis, HlcOrdering, HlcStamp, Ingested, Order, Placed,
    PlacedBreak, SnLane, TimelineRow, Unstamped, Window, timeline,
};
pub use model::tree::KeyTreeSnapshot;
pub use report::{
    AlarmTransition, AlarmsReport, BenchReport, ConformReport, DeclaredEntities, DeclaredEntity,
    DoctorDelta, DoctorReport, EntityKind, ExpectReport, FieldReport, GenPlan, GenPlanEntry,
    GenReport, HealthReport, HelloView, Judgement, LatencyReport, LatencySummary, MeshLink,
    RecordReport, ReplayReport, RouterInfo, SampleRow, SeedCoverage, SeedLeg, ServeSummary,
    ServedCall, Snapshot, SnapshotDiff, SnapshotReport, SnapshotRow, StorageInfo, TimelineReport,
    TopologyEdge, TopologyNode, TopologyReport, WhyReport, ZrecHeader, ZsnapHeader,
    judgement_exit_code,
};
/// The documents the verbs above **return**, at the root beside the verbs
/// themselves — a caller that can spell `run_doctor` can spell what it hands
/// back. The rest of `report` (rows, cells, verdict enums) stays behind
/// `zenkey_fleet::report::*`: it is the rendering vocabulary, and lifting all
/// of it here would make this block a second copy of that module.
pub use report::{
    BindingGraph, CompatReport, ContractAnswer, ContractView, IfaceListing, IfaceView,
    NamespaceListing, PayloadRendering, SchemaView, ServiceListing, ServiceView,
};
pub use report::{
    CondState, PreRollInfo, PreambleInfo, PreambleSemantics, Transition, WatchdogSummary,
};
pub use tape::bench::{BenchSpec, check_bench, run_bench};
pub use tape::ingest::{IngestRow, StreamLine, parse_row, parse_stream_line};
pub use tape::record::{
    PREAMBLE_SKIP_REASON, RecordBounds, ReplayEvent, ReplaySpec, ReplayTarget, SinkCounts,
    VERBATIM, ZREC_READS, ZREC_VERSION, ZrecItem, ZrecReader, ZrecSink, ZrecSource, ZrecWriter,
    excluded_by as zrec_excluded, record, replay,
};
pub use tape::snapshot::{
    SnapshotSpec, Taken, ZSNAP_VERSION, ZsnapReader, ZsnapWriter, report_of as snapshot_report,
    take_snapshot,
};

/// `Send` on the public futures, asserted at compile time (#346).
///
/// Every bus-facing entry point in this crate is awaited from a `tokio::spawn`
/// or an `iced::Task`, both of which require `Send`. Nothing said so: the
/// property held because the frontends happened to compile, and would have
/// broken on the first `Rc` or non-`Send` guard held across an `.await` — at
/// a call site in *another* crate, with the error pointing anywhere but here.
///
/// A `const` block, so it costs nothing at runtime and fails the build here.
#[cfg(test)]
const _: () = {
    const fn assert_send<T: Send>() {}

    #[allow(dead_code)]
    fn engine_futures_are_send() {
        // One per layer, chosen because each holds something across an await
        // that a careless change would make non-`Send`: a session, a lock
        // guard, a store.
        assert_send::<crate::Monitor>();
        assert_send::<crate::Error>();
        assert_send::<crate::BundleStore>();
        assert_send::<crate::Catalog>();
        assert_send::<crate::LensFeed>();
        // A feed holds an event stream and a session across its seeds
        // (#614, FL1).
        assert_send::<crate::PresenceFeed>();
        assert_send::<crate::LivePresence>();
    }

    /// The live presence feed's futures (#614, FL1): a seed holds the
    /// session's read and the event stream across awaits.
    #[allow(dead_code)]
    fn presence_feed_futures_are_send(
        session: &zenoh::Session,
        monitor: &crate::Monitor,
        feed: &mut crate::PresenceFeed,
    ) {
        fn is_send<T: Send>(_: &T) {}
        is_send(&crate::PresenceFeed::open(
            session,
            monitor,
            crate::LivePresence::new(crate::PresenceScope::all()),
            std::time::Duration::ZERO,
        ));
        is_send(&feed.next());
    }

    /// A lens feed following presence (#614, FL4) holds its read's lock, a
    /// session and the store across its descriptor GETs and retrievals.
    #[allow(dead_code)]
    fn lens_feed_futures_are_send(feed: &crate::LensFeed, moved: &[crate::PresenceTransition]) {
        fn is_send<T: Send>(_: &T) {}
        is_send(&feed.on_presence(moved));
    }

    /// The zk2 futures (#612, FJ3): a presence read holds a session and a
    /// `JoinSet` across awaits, a retrieval a lock gate.
    #[allow(dead_code)]
    fn zk2_futures_are_send(
        session: &zenoh::Session,
        store: &crate::BundleStore,
        iface: &zenkey_model::grammar::IfaceId,
        fp: &zenkey_model::canonical::Fingerprint,
    ) {
        fn is_send<T: Send>(_: &T) {}
        let scope = crate::PresenceScope::all();
        is_send(&crate::service_listing(
            session,
            &scope,
            std::time::Duration::ZERO,
        ));
        is_send(&store.fetch(session, iface, fp));
        is_send(&store.fetch_all(session, &[]));
        is_send(&crate::fleet_get(
            session,
            "k",
            &crate::GetOpts::new(std::time::Duration::ZERO),
        ));
        // A health reading holds its window's subscriptions across awaits
        // (#721, PF).
        is_send(&crate::run_health(
            session,
            "",
            crate::HealthTarget::All,
            crate::HealthSpec {
                timeout: std::time::Duration::ZERO,
                window: None,
                grace: std::time::Duration::ZERO,
                clocks_synced: false,
                face: None,
            },
            |_| {},
        ));
        // An alarms reading holds its subscriptions, its queue and its
        // receive path's watch across awaits (#800, AC).
        is_send(&crate::run_alarms(
            session,
            None,
            "",
            crate::AlarmScope::all(),
            crate::AlarmSpec::new(std::time::Duration::ZERO),
            zenkey_model::alarms::Severity::Warning,
            || {},
        ));
        // A telemetry reading holds its subscriptions, its queue and its
        // receive path's watch across awaits (#819, TC).
        let store = crate::BundleStore::new(std::time::Duration::ZERO);
        let offline = crate::ContractSet::new();
        is_send(&crate::run_telemetry(
            session,
            None,
            "",
            crate::TelemetryScope::all(),
            &store,
            &offline,
            crate::TelemetrySpec::new(std::time::Duration::ZERO, std::time::Duration::ZERO),
            |_| {},
        ));
    }

    /// A live alarms reader's futures (#800, AC): the watchdog and a
    /// notifier hold one across a drain.
    #[allow(dead_code)]
    fn alarm_feed_futures_are_send(feed: &mut crate::AlarmFeed) {
        fn is_send<T: Send>(_: &T) {}
        is_send(&feed.next());
        is_send(&feed.refresh_plan().run());
    }
};

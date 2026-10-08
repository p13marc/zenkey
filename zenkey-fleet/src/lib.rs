//! Fleet engine for keyspace-v2 tooling (issue #15).
//!
//! The shared core of `zenctl` and `zengui`: everything a bus explorer needs
//! that is not presentation, in five layers — see **The map** below. The
//! RFC 05 §2.1 fan-in discipline lives in exactly one place
//! ([`bus::query::fleet_get`], moved verbatim from zenctl — target `All`,
//! consolidation `None`, attribution by the reply's own key); the liveliness
//! roster, registry-slice sets, and the schema-aware decode seam build on it.
//!
//! Sessions opened here are deliberately **un-namespaced** (RFC 09 §5): an
//! explorer sees the wire as it really is, full keys included — that is what
//! lets it spot a leak. Do not "fix" this by setting a namespace.
//!
//! **zk2 beside v1** (#612, FJ3). The zk2 core sits beside the v1 engine
//! until FJ9 deletes v1: [`bus::presence`] and [`bus::contracts`] read
//! services, descriptors and contract bundles; [`model::catalog`],
//! [`model::render`] and [`model::structural`] turn them into views and
//! honest renderings; `report`'s `presence`, `iface`, `graph`, `contract`
//! and `payload` domains are what zenctl prints. The zk2 functions take a
//! bare `&Session` and spell base-relative keys, because zk2's resolved
//! verbs read through a session **in** the deployment's namespace (decided
//! 2026-10-08), which ends RFC 09 §5's rule above for zk2; raw verbs and the
//! admin space stay un-namespaced. The runtime is the `zk2` dependency
//! (`zenkey` 0.20 under an alias, so the v1 `zenkey` pin can coexist).
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
//!   `query`, `monitor`, `write`, `serve`, `admin`, `scout`, `seed`, `blob`,
//!   `roster`, `producer`, `body`, and zk2's `presence` and
//!   `contracts`. The RFC 05 §2.1 fan-in
//!   discipline lives here exactly once, in [`bus::query::fleet_get`] (moved
//!   verbatim from zenctl — target `All`, consolidation `None`, attribution
//!   by the reply's own key), and everything in the layer that asks the
//!   fleet a question goes through it. Every liveliness GET goes through
//!   [`bus::presence::liveliness_read`] in the same way, on the unbounded
//!   handler spec §8.1 requires beside a liveliness subscriber (zenoh#2678).
//!   This layer returns observations and never a verdict about one.
//!
//!   Its event sources are **`Stream`s** (#343), not only `recv` loops:
//!   [`EventStream::into_stream`], [`SeededSubscriber`] (a direct impl),
//!   [`RosterWatch::changes`], and a `stream()` on
//!   [`ScoutStream`], [`Responder`], [`MockResponder`] and
//!   [`MatchingEvents`]. The last four borrow rather than consume, because
//!   the `&self` receiver is what lets a query be answered while its stream
//!   is held and a scout be stopped after one; a blanket `impl Stream` would
//!   have taken `&mut self` and spent that. The `recv`/`next` methods stay —
//!   a loop is still the clearer shape for a drain that also selects on
//!   something else, and every consumer in this workspace does.
//!
//!   [`watchdog`] is the one that is not a `Stream` but a
//!   [`Straw`] (#397): it yields transitions *and* returns a
//!   [`WatchdogSummary`], with the acknowledged monitor teardown between the
//!   two. That is a shape `Stream` has no room for, and the only reason this
//!   crate speaks a second streaming vocabulary at all.
//!
//! * **[`model`]** — everything that can do its job from values already in
//!   hand. `facts`, `registry`, `project`, `stats`, `tree`, `skeleton`,
//!   `diff`, `decode`, `retain`, zk2's `catalog` and `render`, the
//!   `structural` ladder both generations fall back to, plus the two
//!   mechanisms every long-running
//!   projection shares (`bounded`, `examples`). Nothing here takes a
//!   session, and that is load-bearing: it is what lets a frontend replay a
//!   `.zrec` through the same projections it runs live.
//!
//! * **[`judge`]** — everything that takes a position. `doctor`, `expect`,
//!   `condition`, `conform`, `field`, `why`, `cutover`, `retired`, `budget`, and
//!   [`judge::common`] for the vocabulary they share. The honesty rules
//!   (RFC 13, v1.24) bite hardest here, so the layer states them once.
//!
//! * **[`report`]** — every serde-pinned wire shape in the crate, split by
//!   domain. Its module doc carries the placement rule, which is the answer
//!   to "where does this struct go?" whenever the struct has a `Serialize`
//!   on it. zk2's shapes are domains of their own (`presence`, `iface`,
//!   `graph`, `contract`, `payload`) beside v1's until FJ9.
//!
//! * **[`tape`]** — traffic as a thing rather than an event. `record`,
//!   `ingest`, `generate`, `synth`, `bench`. It sits beside the others
//!   rather than under them because it both reads from the bus and writes
//!   back to it.
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

// docs.rs builds on nightly with `--cfg docsrs` (see Cargo.toml), which is
// what lets each feature-gated item carry the feature that gates it. Inert
// everywhere else — a stable `cargo doc` never sets the cfg (#325).
//
// **This is inferred, not annotated.** `doc_auto_cfg` was removed in Rust
// 1.92 (rust-lang/rust#138907) by being folded into `doc_cfg`, so enabling
// the feature here labels *every* `#[cfg(feature = "…")]` item, nested
// modules included — verified against the nightly docs.rs uses by rendering
// `judge::doctor`, `bus::body`, `model::decode` and `tape::generate` and
// finding the badge on each. A hand-written
// `#[cfg_attr(docsrs, doc(cfg(…)))]` beside a `#[cfg(…)]` is therefore
// redundant, and a *wrong* one would render a lie; the ones still on the
// re-exports below predate the merge and are harmless.
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
// a module (`zenkey_fleet::model::decode::decode_sample`) is a spelling of the
// same item, never the only way to reach one. The modules stay `pub` because
// their docs are where the reasoning lives and because a reader browsing by
// module should not hit a wall — but nothing supported is *only* there.
//
// Why it matters: both frontends had drifted into a mix of the two
// (`zenkey_fleet::SliceSet` beside `zenkey_fleet::model::decode::SchemaStore`),
// and which spelling a call site used said nothing about how supported the
// item was. With the rule, "is this ours to use?" is answered by looking at
// this block, and adding a public item without adding it here is the omission
// that stands out.
//
// What is deliberately *not* here: `report`'s fifty-odd row and cell types,
// which are the rendering vocabulary rather than the engine's — a frontend
// reaches those through `zenkey_fleet::report::*`, and only the reports the
// verbs below actually **return** are lifted to the root.

#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use bus::body::{
    BodySource, PrepareMode, PrepareSpec, PreparedBody, encode_encoding, prepare_publish,
    prepare_request,
};
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use bus::describe::{DescribeSweep, describe_sweep};
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use judge::condition::{
    AlertAsk, AlertFloor, CondWindow, Condition, DoctorWatch, Eval, RuleSet, RuleState,
    SweepOutcome, WatchdogSpec, watchdog,
};
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use judge::conform::{ConformSpec, run_conform};
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use judge::doctor::{DoctorSpec, run_doctor};
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use judge::expect::{ExpectSpec, QosCheck, run_expect};
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use judge::field::{
    DeclaredPaths, FieldObservation, FieldSpec, KeyFieldContext, KeyFields, PathStats, run_field,
};
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use judge::kind::{KeyKind, KindObservation, judge_kind};
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use model::decode::{
    DEFAULT_MAX_PRODUCERS, DecodedSample, DescribedSchema, Rendering, SchemaStore, Sealed,
    StoreBounds, decode_sample, prewarm, schema_drift, totality_gaps,
};
/// The traits [`watchdog`] is driven through (#397), re-exported so a
/// consumer needs them in scope without taking a direct dependency on
/// `sipper` — and so the version this engine speaks is the one it hands out.
pub use sipper::{Sender, Sipper, Straw};
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use tape::generate::{
    GenPattern, GenSpec, MockProducer, build_plan, run_gen, serve_describe, synthetic_marker,
};
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use tape::synth::Synth;
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use tape::trigger::{TriggerEvent, TriggerSpec, record_on, state_projection};
/// The #159 conformance verdict, re-exported so frontends never reach around
/// the engine for it.
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use zenkey::schema::validate::{NotValidated, Verdict};

pub use bus::admin::{
    AdminEntry, admin_doc_omits_loopback, admin_get, admin_get_within, attach_tokens,
    declared_entities, declared_entities_within, declared_entity_selectors, mesh_links,
    origin_attachments, render_dot, routers, state_coverage, storages, topology,
};
#[cfg(feature = "blob")]
#[cfg_attr(docsrs, doc(cfg(feature = "blob")))]
pub use bus::blob::{BlobFetchSpec, FETCH_PRIORITY, blob_fetch, blob_probe, blob_tree_index};
pub use bus::blob::{BlobTarget, blob_list, declared_by};
pub use bus::monitor::{
    EventStream, FleetEvent, Monitor, MonitorCore, MonitorSpec, SampleSource, SampleView,
    StampProvenance, StreamItem, WatchId,
};
pub use bus::producer::{BringUp, LiveProducer, ReservedError, Responder};
pub use bus::query::{
    Answer, DEFAULT_MAX_REPLIES, FetchOutcome, FetchSpec, FetchedValue, FleetAnswer, GetOpts,
    RegistrySweep, RepeatingQuery, RepeatingRegistry, ServedSlice, SnapshotReplies, StateSample,
    UnreadableReply, declare_repeating, declare_repeating_any, fetch_stored, fetch_value,
    fleet_get, fleet_registry, fleet_registry_by_origin, fleet_registry_raw, snapshot_get,
    state_snapshot,
};
pub use bus::roster::{
    BridgeMatch, RosterChange, RosterWatch, apply_token, bridge_resolve, roster, token_identity,
};
pub use bus::scout::{ScoutStream, scout};
pub use bus::seed::{SeedItem, SeedPolicy, SeededSubscriber, seed_subscribe};
pub use bus::serve::{MockResponder, ServedQuery, declare_responder};
pub use bus::session::{
    Fleet, OPEN_TIMEOUT, OpenFailure, open, open_in_namespace, open_reporting,
    open_reporting_within, open_with_config,
};
pub use bus::write::{
    CallSpec, CallTarget, MatchingEvents, Publication, RetireClass, TraceSpec, WriteAct, call,
    call_traced, check_concrete, check_fanout, check_retire, declare_publication,
};
pub use judge::budget::BudgetObservation;
pub use judge::common::{EXPANSION_CAP, data_plane_scopes, new_prefix};
pub use judge::doctor_delta::doctor_delta;
pub use judge::self_stats::{SelfStats, TableStats, judge_self_stats, read_self_stats};
// Types reachable *through* root-exported ones — a caller that matches on
// `KeyShape::V1` or walks a `Skeleton` needs these, and had to spell a module
// path to name them (#350).
pub use model::bounded::DEFAULT_MAX_KEYS;
pub use model::facts::{ClassKind, OriginKind, SubjectFacts, V1Facts};
pub use model::registry::{SliceSource, UnionOutcome};
pub use model::skeleton::{
    DeclRef, Evidence, NodeStats, SkeletonChunk, SkeletonCoverage, SkeletonNode, merge,
};
pub use model::tree::{TreeNode, TreeRow, TreeRows};
// The rest of what the frontends actually reach for. The structural ladder
// needs no codec, so it is no longer gated on `decode` (#612, FJ3).
pub use model::structural::{OBSERVE_LIMIT, structural, structural_value};
// zk2 (#612, FJ3): presence and its §8.1 liveliness chokepoint, contract
// retrieval, the catalog and its offline contracts, and honest rendering.
// The generic verbs keep a zk2 name at the root — `observe` and `render`
// alone would read as the crate's.
pub use bus::contracts::{BundleStore, DEFAULT_MAX_CONTRACTS, UNAVAILABLE_TTL};
pub use bus::presence::{
    DESCRIBE_CONCURRENCY, NAMESPACE_SELECTOR, Scope as PresenceScope,
    describe as describe_instances, liveliness_read, namespace_listing,
    observe as observe_presence, read_tokens, service_listing,
};
// `Observed` is `model::export`'s at the root already; zk2's is the
// presence read, and says so.
pub use model::catalog::{
    Catalog, ContractSet, ContractState, Contracts, DescriptorRead, LoadProblem,
    Observed as ObservedPresence, Revision, namespaces, type_view,
};
pub use model::compat::compat;
pub use model::render::{
    Member, render as render_payload, render_with as render_payload_with, resolved_revision,
};
pub use tape::record::{rfc3339_from_unix, rfc3339_now};
// The judging vocabulary a caller can drive directly (#349's evidence
// structs among them).
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use judge::condition::{
    SilenceEvidence, TickEvidence, judge_alert_firing, judge_doctor_check, judge_origin_down,
};
pub use judge::retired::EntryEvidence;
// The remaining items a frontend actually calls. Every one of these was
// reachable only by module path (#350) — which said nothing about whether it
// was ours to use.
pub use bus::teardown::DECLARE_TIMEOUT;
pub use error::{BoxedCause, Error, Result, one_line};
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use judge::field::DEFAULT_MAX_PATHS;
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use judge::why::is_cause;
// The two scope notes keep their own names rather than one: they are two
// different O5 statements about two different windows, which is why
// `judge/common.rs` declined to merge them. A name collision is not a reason
// for an item to be unreachable from the root, though (#350).
pub use judge::cutover::run_cutover;
pub use judge::cutover::scope_note as cutover_scope_note;
pub use judge::retired::run_retired;
pub use judge::retired::scope_note as retired_scope_note;
pub use judge::why::{StoredLookup, StoredValue, WhyInputs, WhySpec, WireWatch, run_why};
// `diff` is `value_diff` at the root: a bare `diff` beside `byte_diff` in a
// crate that also has `schema_drift` and `slice::diff` reads as *the* diff.
pub use model::acl::{
    AclOptions, check_acl, explain_acl, plan_acl, plan_face, to_json5 as acl_plan_json5,
};
pub use model::alert::alert_transition;
pub use model::diff::{ByteDiff, Change, ValueDiff, byte_diff, diff as value_diff};
pub use model::export::{
    DEFAULT_MAX_SERIES, DoctorRun, ExportLedger, FIELD_CAP, FoldInputs, Observed, PayloadVerdict,
    WILDCARD_EXCLUDES, excluded_by,
};
pub use model::facts::{
    FactsCache, KeyDescription, KeyFacts, KeyShape, Registration, describe_key,
};
pub use model::impact::{ImpactInputs, MAX_DEPTH_CAP, attribute, entity_of};
pub use model::origin_map::{Label, MapError, MapPlan, OriginProfile, origin_profiles, plan_map};
pub use model::prom::{exposition, metric_name};
pub use model::registry::SliceSet;
pub use model::retain::{RetentionBudget, RetentionStats};
pub use model::skeleton::{MergedNode, NodeStatus, Skeleton};
pub use model::snapshot::{fold_latest, holder_of, registration_of, stamper_of};
pub use model::snapshot_diff::{DiffOpts, diff_normalized, diff_snapshots};
pub use model::stats::{KeyStats, StampClass, StatsTable};
pub use model::storage::{
    check_storages, explain as explain_storage, plan_storages, to_json5 as storage_plan_json5,
};
pub use model::timeline::{
    ArrivalAxis, ArrivalOrdering, Break, HlcAxis, HlcOrdering, HlcStamp, Ingested, Order, Placed,
    PlacedBreak, SnLane, TimelineRow, Unstamped, Window, timeline,
};
pub use model::tree::KeyTreeSnapshot;
pub use report::{
    AlertState, AlertTransition, AliasDoc, BenchReport, CallReport, CollapsedProducer,
    ConformReport, Coverage, CoverageRow, CutoverReport, DeclaredEntities, DeclaredEntity,
    DoctorDelta, DoctorReport, DriftVerdict, EdgeDoc, EdgeEnd, EdgeKind, EntityDoc, EntityKind,
    ExpectReport, ExportSnapshot, Fault, FieldReport, GenPlanEntry, GenReport, HelloView,
    ImpactReport, Judgement, LatencyReport, LatencySummary, MeshLink, OriginAttachment,
    RecordReport, RenderSource, ReplayReport, RetiredReport, RouterInfo, Rung, RungAnswer,
    SampleRow, SchemaDrift, SchemaServer, SeedCoverage, Snapshot, SnapshotDiff, SnapshotReport,
    SnapshotRow, StorageInfo, TimelineReport, TopologyEdge, TopologyNode, TopologyReport,
    TotalityGap, TraceReport, ValueSource, WhyReport, WhyVerdict, ZrecHeader, ZsnapHeader,
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
// `CondState` and `Transition` are unconditional since v1.34: a version-2
// `.zrec` carries the trigger record, and the reader is not decode-gated.
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use report::WatchdogSummary;
pub use report::{CondState, PreRollInfo, PreambleInfo, PreambleSemantics, Transition};
pub use tape::bench::{BenchSpec, run_bench};
pub use tape::ingest::{IngestRow, StreamLine, parse_row, parse_stream_line};
pub use tape::record::{
    PREAMBLE_SKIP_REASON, RecordBounds, ReplayEvent, ReplaySpec, ReplayTarget, SinkCounts,
    ZREC_READS, ZREC_VERSION, ZrecItem, ZrecReader, ZrecSink, ZrecSource, ZrecWriter, record,
    replay,
};
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use tape::snapshot::{SnapshotSpec, Taken, take_snapshot};
pub use tape::snapshot::{ZSNAP_VERSION, ZsnapReader, ZsnapWriter, report_of as snapshot_report};
/// The RFC 07 reference client, re-exported so a frontend, an example or a
/// test cannot end up on a different version of it than the engine.
#[cfg(feature = "blob")]
#[cfg_attr(docsrs, doc(cfg(feature = "blob")))]
pub use zblob;

/// `Send` on the public futures, asserted at compile time (#346).
///
/// Every bus-facing entry point in this crate is awaited from a `tokio::spawn`
/// or an `iced::Task`, both of which require `Send`. Nothing said so: the
/// property held because `zengui` happens to use iced, and would have broken
/// on the first `Rc` or non-`Send` guard held across an `.await` — at a call
/// site in *another* crate, with the error pointing anywhere but here.
///
/// A `const` block, so it costs nothing at runtime and fails the build here.
#[cfg(all(test, feature = "decode"))]
const _: () = {
    const fn assert_send<T: Send>() {}

    #[allow(dead_code)]
    fn engine_futures_are_send() {
        // One per layer, chosen because each holds something across an await
        // that a careless change would make non-`Send`: a session, a lock
        // guard, a decoder registry.
        assert_send::<crate::Fleet<'_>>();
        assert_send::<crate::SliceSet>();
        assert_send::<crate::SchemaStore>();
        assert_send::<crate::Monitor>();
        assert_send::<crate::Error>();
        assert_send::<crate::BundleStore>();
        assert_send::<crate::Catalog>();
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
    }
};

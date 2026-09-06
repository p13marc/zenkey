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
//!   `roster`, `discover`, `producer`, `body`. The RFC 05 §2.1 fan-in
//!   discipline lives here exactly once, in [`bus::query::fleet_get`] (moved
//!   verbatim from zenctl — target `All`, consolidation `None`, attribution
//!   by the reply's own key), and everything in the layer that asks the
//!   fleet a question goes through it. This layer returns observations and
//!   never a verdict about one.
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
//!   `diff`, `decode`, `retain`, plus the two mechanisms every long-running
//!   projection shares (`bounded`, `examples`). Nothing here takes a
//!   session, and that is load-bearing: it is what lets a frontend replay a
//!   `.zrec` through the same projections it runs live.
//!
//! * **[`judge`]** — everything that takes a position. `doctor`, `expect`,
//!   `condition`, `field`, `why`, `cutover`, `retired`, `budget`, and
//!   [`judge::common`] for the vocabulary they share. The honesty rules
//!   (RFC 13, v1.24) bite hardest here, so the layer states them once.
//!
//! * **[`report`]** — every serde-pinned wire shape in the crate, split by
//!   domain. Its module doc carries the placement rule, which is the answer
//!   to "where does this struct go?" whenever the struct has a `Serialize`
//!   on it.
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
    CondWindow, Condition, DoctorWatch, Eval, RuleState, WatchdogSpec, watchdog,
};
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
    StoreBounds, decode_sample, prewarm, schema_drift, schema_dump, schema_rows_for_type,
    totality_gaps,
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
/// The #159 conformance verdict, re-exported so frontends never reach around
/// the engine for it.
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use zenkey::schema::validate::{NotValidated, Verdict};

pub use bus::admin::{
    AdminEntry, admin_doc_omits_loopback, admin_get, admin_get_within, declared_entities,
    mesh_links, origin_attachments, render_dot, routers, state_coverage, storages, topology,
};
#[cfg(feature = "blob")]
#[cfg_attr(docsrs, doc(cfg(feature = "blob")))]
pub use bus::blob::{BlobFetchSpec, FETCH_PRIORITY, blob_fetch, blob_probe, blob_tree_index};
pub use bus::blob::{BlobTarget, blob_list, declared_by};
pub use bus::discover::{AliveToken, discover_bases};
pub use bus::monitor::{
    EventStream, FleetEvent, Monitor, MonitorCore, MonitorSpec, SampleSource, SampleView,
    StampProvenance, StreamItem, WatchId,
};
pub use bus::producer::{BringUp, LiveProducer, ReservedError, Responder};
pub use bus::query::{
    Answer, DEFAULT_MAX_REPLIES, FetchOutcome, FetchSpec, FetchedValue, FleetAnswer, GetOpts,
    RepeatingQuery, RepeatingRegistry, ServedSlice, StateSample, declare_repeating,
    declare_repeating_any, fetch_stored, fetch_value, fleet_get, fleet_registry,
    fleet_registry_by_origin, fleet_registry_raw, state_snapshot,
};
pub use bus::roster::{
    BridgeMatch, RosterChange, RosterWatch, apply_token, bridge_resolve, node_info, node_rows,
    roster, token_identity,
};
pub use bus::scout::{ScoutStream, scout};
pub use bus::seed::{SeedItem, SeedPolicy, SeededSubscriber, seed_subscribe};
pub use bus::serve::{MockResponder, ServedQuery, declare_responder};
pub use bus::session::{
    Fleet, OPEN_TIMEOUT, OpenFailure, open, open_reporting, open_reporting_within, open_with_config,
};
pub use bus::write::{
    CallSpec, CallTarget, MatchingEvents, Publication, RetireClass, call, check_retire,
    declare_publication,
};
pub use judge::budget::{BudgetObservation, join_budget};
pub use judge::common::{EXPANSION_CAP, data_plane_scopes, new_prefix};
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
// The rest of what the frontends actually reach for.
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use model::decode::{OBSERVE_LIMIT, structural, structural_value};
pub use tape::record::rfc3339_now;
// The judging vocabulary a caller can drive directly (#349's evidence
// structs among them).
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use judge::condition::{SilenceEvidence, TickEvidence, judge_doctor_check, judge_origin_down};
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
pub use model::diff::{ByteDiff, Change, ValueDiff, byte_diff, diff as value_diff};
pub use model::facts::{
    FactsCache, KeyDescription, KeyFacts, KeyShape, Registration, describe_key,
};
pub use model::registry::SliceSet;
pub use model::retain::{RetentionBudget, RetentionStats};
pub use model::skeleton::{MergedNode, NodeStatus, Skeleton};
pub use model::stats::{KeyStats, StampClass, StatsTable};
pub use model::tree::KeyTreeSnapshot;
/// The documents the verbs above **return**, at the root beside the verbs
/// themselves — a caller that can spell `run_doctor` can spell what it hands
/// back. The rest of `report` (rows, cells, verdict enums) stays behind
/// `zenkey_fleet::report::*`: it is the rendering vocabulary, and lifting all
/// of it here would make this block a second copy of that module.
pub use report::{
    BenchReport, CallReport, CollapsedProducer, Coverage, CoverageRow, CutoverReport,
    DeclaredEntities, DeclaredEntity, DiscoveredBase, DoctorReport, DriftVerdict, EntityKind,
    ExpectReport, Fault, FieldReport, Freshness, GenPlanEntry, GenReport, HelloView, Judgement,
    LatencyReport, LatencySummary, MeshLink, NodeInfo, OriginAttachment, ProducerInfo,
    RecordReport, ReplayReport, RetiredReport, RouterInfo, Rung, RungAnswer, SampleRow,
    SchemaDrift, SchemaServer, SeedCoverage, StorageInfo, TopologyEdge, TopologyNode,
    TopologyReport, TotalityGap, ValueSource, WhyReport, WhyVerdict, ZrecHeader,
    judgement_exit_code,
};
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub use report::{CondState, Transition, WatchdogSummary};
pub use tape::bench::{BenchSpec, run_bench};
pub use tape::ingest::{IngestRow, StreamLine, parse_row, parse_stream_line};
pub use tape::record::{
    RecordBounds, ReplayEvent, ReplaySpec, ReplayTarget, ZREC_VERSION, ZrecItem, ZrecReader,
    ZrecSink, ZrecSource, ZrecWriter, record, replay,
};
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
    }
};

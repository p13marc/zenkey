//! Fleet engine for keyspace-v2 tooling (issue #15).
//!
//! The shared core of `zenctl` and `zengui`: everything a bus explorer needs
//! that is not presentation. The RFC 05 §2.1 fan-in discipline lives in
//! exactly one place ([`bus::query::fleet_get`], moved verbatim from zenctl —
//! target `All`, consolidation `None`, attribution by the reply's own key);
//! the liveliness roster, registry-slice sets, and the schema-aware decode
//! seam build on it.
//!
//! Sessions opened here are deliberately **un-namespaced** (RFC 09 §5): an
//! explorer sees the wire as it really is, full keys included — that is what
//! lets it spot a leak. Do not "fix" this by setting a namespace.

pub mod bus;
pub mod judge;
pub mod model;
pub mod report;
pub mod tape;
// ─── the supported surface ──────────────────────────────────────────────────
//
// **The rule: the crate root is the whole supported surface.** Every type and
// function a frontend is meant to use is re-exported here, and a path through
// a module (`zenkey_fleet::decode::decode_sample`) is a spelling of the same
// item, never the only way to reach one. The modules stay `pub` because their
// docs are where the reasoning lives and because a reader browsing by module
// should not hit a wall — but nothing supported is *only* there.
//
// Why it matters: both frontends had drifted into a mix of the two
// (`zenkey_fleet::SliceSet` beside `zenkey_fleet::decode::SchemaStore`), and
// which spelling a call site used said nothing about how supported the item
// was. With the rule, "is this ours to use?" is answered by looking at this
// block, and adding a public item without adding it here is the omission that
// stands out.
//
// What is deliberately *not* here: `report`'s fifty-odd row and cell types,
// which are the rendering vocabulary rather than the engine's — a frontend
// reaches those through `zenkey_fleet::report::*`, and only the reports the
// verbs below actually **return** are lifted to the root.

#[cfg(feature = "decode")]
pub use bus::body::{
    BodySource, PrepareMode, PrepareSpec, PreparedBody, encode_encoding, prepare_publish,
    prepare_request,
};
#[cfg(feature = "decode")]
pub use judge::condition::{
    CondWindow, Condition, DoctorWatch, Eval, RuleState, WatchdogSpec, run_watchdog,
};
#[cfg(feature = "decode")]
pub use judge::doctor::{DoctorSpec, run_doctor};
#[cfg(feature = "decode")]
pub use judge::expect::{ExpectSpec, QosCheck, run_expect};
#[cfg(feature = "decode")]
pub use judge::field::{DeclaredPaths, FieldObservation, FieldSpec, KeyFieldContext, run_field};
#[cfg(feature = "decode")]
pub use model::decode::{
    DecodedSample, Rendering, SchemaStore, decode_sample, schema_drift, schema_dump,
    schemas_for_type, totality_gaps,
};
#[cfg(feature = "decode")]
pub use tape::generate::{
    GenPattern, GenSpec, MockProducer, build_plan, run_gen, serve_describe, synthetic_marker,
};
#[cfg(feature = "decode")]
pub use tape::synth::Synth;
/// The #159 conformance verdict, re-exported so frontends never reach around
/// the engine for it.
#[cfg(feature = "decode")]
pub use zenkey::schema::validate::{NotValidated, Verdict};

pub use bus::admin::{
    AdminEntry, admin_doc_omits_loopback, admin_get, declared_entities, mesh_links,
    origin_attachments, render_dot, routers, state_coverage, storages, topology,
};
#[cfg(feature = "blob")]
pub use bus::blob::{BlobFetchSpec, FETCH_PRIORITY, blob_fetch, blob_probe, blob_tree_index};
pub use bus::blob::{BlobTarget, blob_list, declared_by};
pub use bus::discover::{AliveToken, discover_bases};
pub use bus::monitor::{
    EventStream, FleetEvent, Monitor, MonitorCore, MonitorSpec, SampleSource, SampleView,
    StampProvenance, StreamItem, WatchId,
};
pub use bus::producer::{BringUp, LiveProducer, ReservedError, Responder};
pub use bus::query::{
    Answer, FetchOutcome, FetchSpec, FetchedValue, FleetAnswer, GetOpts, RepeatingQuery,
    RepeatingRegistry, StateSample, declare_repeating, declare_repeating_any, fetch_stored,
    fetch_value, fleet_get, fleet_registry, state_snapshot,
};
pub use bus::roster::{
    BridgeMatch, RosterChange, RosterWatch, apply_token, bridge_resolve, node_info, node_rows,
    roster, token_identity,
};
pub use bus::scout::{ScoutStream, scout};
pub use bus::seed::{SeedItem, SeedPolicy, SeededSubscriber, seed_subscribe};
pub use bus::serve::{MockResponder, ServedQuery, declare_responder};
pub use bus::session::{Fleet, OpenFailure, open, open_reporting, open_with_config};
pub use bus::write::{
    CallSpec, CallTarget, MatchingEvents, Publication, RetireClass, call, check_retire,
    declare_publication,
};
pub use judge::budget::{BudgetObservation, join_budget};
pub use judge::common::{CHECK_IDS, EXPANSION_CAP, RUNG_IDS, data_plane_scopes, new_prefix};
pub use judge::cutover::run_cutover;
pub use judge::retired::run_retired;
pub use judge::why::{WhyInputs, WhySpec, run_why};
pub use model::diff::{ByteDiff, Change, ValueDiff, byte_diff};
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
    BenchReport, CallReport, Coverage, CoverageRow, CutoverReport, DeclaredEntities,
    DeclaredEntity, DiscoveredBase, DoctorReport, EntityKind, ExpectReport, Fault, FieldReport,
    Freshness, GenPlanEntry, GenReport, HelloView, Judgement, LatencyReport, LatencySummary,
    MeshLink, NodeInfo, OriginAttachment, ProducerInfo, RecordReport, ReplayReport, RetiredReport,
    RouterInfo, Rung, RungAnswer, SampleRow, SchemaDrift, SeedCoverage, StorageInfo, TopologyEdge,
    TopologyNode, TopologyReport, TotalityGap, ValueSource, WhyReport, WhyVerdict, ZrecHeader,
    judgement_exit_code,
};
#[cfg(feature = "decode")]
pub use report::{CondState, Transition, WatchdogSummary};
pub use tape::bench::{BenchSpec, run_bench};
pub use tape::ingest::{IngestRow, StreamLine, parse_row, parse_stream_line};
pub use tape::record::{
    RecordBounds, ReplayEvent, ReplaySpec, ReplayTarget, ZREC_VERSION, ZrecItem, ZrecReader,
    ZrecWriter, record, replay,
};
/// The RFC 07 reference client, re-exported so a frontend, an example or a
/// test cannot end up on a different version of it than the engine.
#[cfg(feature = "blob")]
pub use zblob;

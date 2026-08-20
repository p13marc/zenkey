//! Fleet engine for keyspace-v2 tooling (issue #15).
//!
//! The shared core of `zenctl` and `zengui`: everything a bus explorer needs
//! that is not presentation. The RFC 05 §2.1 fan-in discipline lives in
//! exactly one place ([`query::fleet_get`], moved verbatim from zenctl —
//! target `All`, consolidation `None`, attribution by the reply's own key);
//! the liveliness roster, registry-slice sets, and the schema-aware decode
//! seam build on it.
//!
//! Sessions opened here are deliberately **un-namespaced** (RFC 09 §5): an
//! explorer sees the wire as it really is, full keys included — that is what
//! lets it spot a leak. Do not "fix" this by setting a namespace.

pub mod admin;
pub mod bench;
pub mod blob;
pub mod context_store;
pub mod cutover;
pub mod diff;
pub mod discover;
pub mod facts;
pub mod ingest;
pub mod project;
pub mod query;
pub mod record;
pub mod registry;
pub mod report;
pub mod roster;
pub mod scout;
pub mod seed;
pub mod serve;
pub mod session;
pub mod skeleton;
pub mod stats;
pub mod sub;
pub mod tree;
pub mod write;

#[cfg(feature = "decode")]
pub mod body;
#[cfg(feature = "decode")]
pub mod decode;
#[cfg(feature = "decode")]
pub mod doctor;
#[cfg(feature = "decode")]
pub mod expect;
#[cfg(feature = "decode")]
pub mod generate;
#[cfg(feature = "decode")]
pub mod synth;
#[cfg(feature = "decode")]
pub use body::{
    BodySource, PrepareMode, PreparedBody, encode_encoding, prepare_publish, prepare_request,
};
#[cfg(feature = "decode")]
pub use decode::{
    DecodedSample, SchemaDrift, TotalityGap, schema_drift, schema_dump, schemas_for_type,
    totality_gaps,
};
#[cfg(feature = "decode")]
pub use doctor::{CHECK_IDS, DoctorSpec, run_doctor};
#[cfg(feature = "decode")]
pub use expect::{ExpectSpec, QosCheck, run_expect};
/// The #159 conformance verdict, re-exported so frontends never reach around
/// the engine for it.
#[cfg(feature = "decode")]
pub use zenkey::schema::validate::{NotValidated, Verdict};

pub use admin::{
    AdminEntry, Coverage, CoverageRow, RouterInfo, StorageInfo, admin_get, routers, state_coverage,
    storages,
};
pub use admin::{DeclaredEntities, DeclaredEntity, EntityKind, declared_entities};
pub use admin::{
    MeshLink, OriginAttachment, TopologyEdge, TopologyNode, TopologyReport, mesh_links,
    origin_attachments, render_dot, topology,
};
pub use bench::{BenchSpec, bench_rpc};
#[cfg(feature = "blob")]
pub use blob::{BlobFetchSpec, FETCH_PRIORITY, blob_fetch, blob_probe, blob_tree_index};
pub use blob::{BlobTarget, blob_list, declared_by};
pub use context_store::{StoredContext, active_name, cache_dir};
pub use cutover::run_cutover;
pub use diff::{ByteDiff, Change, ValueDiff, byte_diff};
pub use discover::{AliveToken, DiscoveredBase, discover_bases};
pub use facts::{FactsCache, KeyDescription, KeyFacts, KeyShape, Registration, describe_key};
pub use ingest::{IngestRow, parse_row};
pub use query::{
    Answer, FetchOutcome, FetchSpec, FetchedValue, FleetAnswer, RepeatingQuery, RepeatingRegistry,
    StateSample, ValueSource, declare_repeating, declare_repeating_any, fetch_value, fleet_get,
    fleet_get_at, fleet_get_call, fleet_registry, state_snapshot,
};
pub use record::{
    RecordBounds, RecordReport, ReplayEvent, ReplayReport, ReplayTarget, ZREC_VERSION, ZrecHeader,
    ZrecItem, ZrecReader, ZrecWriter, record, replay,
};
pub use registry::SliceSet;
pub use roster::{
    BridgeMatch, Freshness, NodeInfo, ProducerInfo, RosterChange, RosterWatch, apply_token,
    bridge_resolve, node_info, node_rows, roster, token_identity,
};
pub use scout::{HelloView, ScoutStream, scout};
pub use seed::{SeedCoverage, SeedItem, SeedPolicy, SeededSubscriber, seed_subscribe};
pub use serve::{MockResponder, ServedQuery, declare_responder};
pub use session::{OpenFailure, open, open_reporting, open_with_config};
pub use skeleton::{MergedNode, NodeStatus, Skeleton};
pub use stats::{LatencyReport, LatencySummary, StampClass};
pub use sub::{
    EventStream, FleetEvent, Monitor, MonitorCore, MonitorSpec, SampleSource, SampleView,
    StampProvenance, StreamItem, WatchId,
};
pub use tree::KeyTreeSnapshot;
pub use write::{
    CallTarget, MatchingEvents, Publication, RetireClass, call, check_retire, declare_publication,
};
/// The RFC 07 reference client, re-exported so a frontend, an example or a
/// test cannot end up on a different version of it than the engine.
#[cfg(feature = "blob")]
pub use zblob;

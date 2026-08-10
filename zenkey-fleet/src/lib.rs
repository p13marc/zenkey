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
pub mod diff;
pub mod discover;
pub mod facts;
pub mod query;
pub mod registry;
pub mod report;
pub mod roster;
pub mod seed;
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
pub use body::{
    BodySource, PrepareMode, PreparedBody, encode_encoding, prepare_publish, prepare_request,
};
#[cfg(feature = "decode")]
pub use decode::{
    SchemaDrift, TotalityGap, schema_drift, schema_dump, schemas_for_type, totality_gaps,
};
#[cfg(feature = "decode")]
pub use doctor::{CHECK_IDS, DoctorSpec, run_doctor};

pub use admin::{
    AdminEntry, Coverage, CoverageRow, RouterInfo, StorageInfo, admin_get, routers, state_coverage,
    storages,
};
pub use admin::{DeclaredEntities, DeclaredEntity, EntityKind, declared_entities};
pub use bench::{BenchSpec, bench_rpc};
#[cfg(feature = "blob")]
pub use blob::{BlobFetchSpec, FETCH_PRIORITY, blob_fetch, blob_probe};
pub use blob::{BlobTarget, blob_list, declared_by};
pub use context_store::{StoredContext, active_name, cache_dir};
pub use diff::{ByteDiff, Change, ValueDiff, byte_diff};
pub use discover::{AliveToken, DiscoveredBase, discover_bases};
pub use facts::{KeyDescription, KeyFacts, KeyShape, Registration, describe_key};
pub use query::{
    Answer, FetchOutcome, FetchSpec, FetchedValue, FleetAnswer, RepeatingQuery, RepeatingRegistry,
    StateSample, ValueSource, declare_repeating, declare_repeating_any, fetch_value, fleet_get,
    fleet_get_at, fleet_registry, state_snapshot,
};
pub use registry::SliceSet;
pub use roster::{Freshness, NodeInfo, ProducerInfo, node_info, roster};
pub use seed::{SeedCoverage, SeedItem, SeedPolicy, SeededSubscriber, seed_subscribe};
pub use session::open;
pub use skeleton::{MergedNode, NodeStatus, Skeleton};
pub use sub::{
    EventStream, FleetEvent, Monitor, MonitorCore, MonitorSpec, SampleView, StreamItem, WatchId,
};
pub use tree::KeyTreeSnapshot;
pub use write::{CallTarget, MatchingEvents, Publication, call, declare_publication};
/// The RFC 07 reference client, re-exported so a frontend, an example or a
/// test cannot end up on a different version of it than the engine.
#[cfg(feature = "blob")]
pub use zblob;

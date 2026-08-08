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
pub mod context_store;
pub mod discover;
pub mod facts;
pub mod query;
pub mod registry;
pub mod report;
pub mod roster;
pub mod session;
pub mod skeleton;
pub mod stats;
pub mod sub;
pub mod tree;
pub mod write;

#[cfg(feature = "decode")]
pub mod decode;

pub use admin::{
    AdminEntry, Coverage, CoverageRow, RouterInfo, StorageInfo, admin_get, routers, state_coverage,
    storages,
};
pub use admin::{DeclaredEntities, DeclaredEntity, EntityKind, declared_entities};
pub use context_store::StoredContext;
pub use discover::{AliveToken, DiscoveredBase, discover_bases};
pub use facts::{KeyDescription, KeyFacts, KeyShape, Registration, describe_key};
pub use query::{
    Answer, FetchOutcome, FetchSpec, FetchedValue, FleetAnswer, StateSample, ValueSource,
    fetch_value, fleet_get, fleet_registry, state_snapshot,
};
pub use registry::SliceSet;
pub use roster::{Freshness, NodeInfo, ProducerInfo, node_info, roster};
pub use session::open;
pub use skeleton::{MergedNode, NodeStatus, Skeleton};
pub use sub::{
    EventStream, FleetEvent, Monitor, MonitorCore, MonitorSpec, SampleView, StreamItem, WatchId,
};
pub use tree::KeyTreeSnapshot;
pub use write::{CallTarget, Publication, call, declare_publication};

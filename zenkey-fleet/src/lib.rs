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

pub mod query;
pub mod registry;
pub mod roster;
pub mod session;
pub mod stats;
pub mod sub;
pub mod tree;

#[cfg(feature = "decode")]
pub mod decode;

pub use query::{Answer, FleetAnswer, fleet_get, fleet_registry};
pub use registry::SliceSet;
pub use roster::roster;
pub use session::open;
pub use sub::{EventStream, FleetEvent, Monitor, MonitorCore, MonitorSpec, SampleView, StreamItem};
pub use tree::KeyTreeSnapshot;

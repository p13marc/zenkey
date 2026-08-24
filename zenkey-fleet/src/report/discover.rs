//! The discovery plane: a deployment base seen on the wire, and how it was
//! seen.
//!
//! Un-namespaced observation is the whole point (RFC 09 §5): an explorer
//! that had set a namespace could not have found any base but its own.

use std::collections::BTreeSet;

use serde::Serialize;

/// One discovered base and the evidence for it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DiscoveredBase {
    /// `""` is the empty base (keys start at `v1/` on the wire).
    pub base: String,
    /// Origins holding alive tokens under this base.
    pub origins: BTreeSet<String>,
    /// Producer names alive under this base.
    pub producers: BTreeSet<String>,
    /// Storages whose config names this base, as `name@zid`.
    pub storages: Vec<String>,
}

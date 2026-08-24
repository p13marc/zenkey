//! The scouting plane (RFC 09 §5): what a UDP scout heard, as a document.
//! Opt-in, and the report says so — an explorer that scouted by default
//! would be changing the bus it came to observe.

use serde::Serialize;

/// What a scouting round heard, and what it could have heard (#236).
///
/// A wrapper rather than a bare `Vec<HelloView>` for one reason, and it is
/// RFC 09 §5.1 **O5**: scouting reaches the local multicast domain and the
/// configured gossip, and nothing else. An empty list is a **boundary**, not a
/// claim that nothing is running — and a bare array cannot say so, which is
/// why `zenctl scout --format json` used to print that sentence as *prose on
/// stdout* where a document was promised.
#[derive(Debug, Clone, Serialize)]
pub struct ScoutReport {
    /// The node kinds asked for; empty means all three.
    pub asked: Vec<String>,
    /// How long the round listened.
    pub timeout_s: f64,
    /// Distinct nodes heard, first sighting winning — a census, not an
    /// arrival log.
    pub heard: Vec<crate::report::HelloView>,
}

/// One Hello, owned — zenoh's [`Hello`](zenoh::scouting::Hello) is a wrapper we flatten so callers
/// (a CLI row, a widget) hold plain strings, serialized as they render.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HelloView {
    /// The node's Zenoh id, hex-formatted.
    pub zid: String,
    /// What the node says it is: `router`, `peer`, or `client`.
    pub whatami: String,
    /// The locators the node advertises, verbatim.
    pub locators: Vec<String>,
}

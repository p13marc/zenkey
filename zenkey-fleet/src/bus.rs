//! **Layer 1 — the bus.** Everything that holds a session and talks to Zenoh.
//!
//! One rule places a module here: *it needs a live session to do its job*.
//! Every function below either takes a [`crate::Fleet`] (base-aware) or a
//! bare `&Session` (base-less, and saying so), and everything it returns is
//! an observation — never a judgement about one. A module that can compute
//! its answer from values already in hand belongs in [`crate::model`]; a
//! module that turns an observation into a verdict belongs in
//! [`crate::judge`].
//!
//! The RFC 05 §2.1 fan-in discipline lives here exactly once, in
//! [`query::fleet_get`] — target `All`, consolidation `None`, attribution by
//! the reply's own key. Everything in this layer that asks the fleet a
//! question goes through it rather than reaching for `session.get`, which is
//! what makes "silence is never a verdict" a property of the crate instead of
//! a habit of its authors.
//!
//! Sessions opened here are deliberately **un-namespaced** (RFC 09 §5): an
//! explorer sees the wire as it really is, full keys included — that is what
//! lets it spot a leak. Do not "fix" this by setting a namespace.

pub mod admin;
pub mod blob;
pub mod discover;
pub mod monitor;
pub mod producer;
pub mod query;
pub mod roster;
pub mod scout;
pub mod seed;
pub mod serve;
pub mod session;
pub(crate) mod teardown;
pub mod write;

#[cfg(feature = "decode")]
pub mod body;

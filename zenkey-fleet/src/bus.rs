//! **Layer 1 — the bus.** Everything that holds a session and talks to Zenoh.
//!
//! One rule places a module here: *it needs a live session to do its job*.
//! Every function below takes a `&Session`, and everything it returns is an
//! observation — never a judgement about one. A module that can compute its
//! answer from values already in hand belongs in [`crate::model`]; a module
//! that turns an observation into a verdict belongs in [`crate::judge`].
//!
//! The RFC 05 §2.1 fan-in discipline lives here exactly once, in
//! [`query::fleet_get`] — target `All`, consolidation `None`, attribution by
//! the reply's own key. Everything in this layer that asks a raw question
//! goes through it rather than reaching for `session.get`, which is what
//! makes "silence is never a verdict" a property of the crate instead of a
//! habit of its authors.
//!
//! Every **liveliness** GET goes through [`presence::liveliness_read`] the
//! same way: it runs on the unbounded handler spec §8.1 requires on a
//! session that holds a liveliness subscriber (zenoh#2678).
//!
//! **Which session.** Decided 2026-10-08: a resolved verb reads through a
//! session opened **in** the deployment's namespace, and the modules it
//! reads through spell base-relative `zk2/…` keys — [`presence`] (services
//! from tokens and descriptors), [`contracts`] (bundle retrieval, §8.4),
//! [`operation`] (a call through the runtime's `Client` or `Fleet`, §5.1)
//! and [`consume`] (state and subscriptions through its `Consumer`, an
//! archive's last-known state, §3.2, §4). A raw observer — [`monitor`],
//! [`query`], [`admin`], [`scout`] — runs in **no** namespace, and sees the
//! wire as it really is, full keys included: that is what lets it spot a
//! key outside the deployment. [`lens`] is the bridge: what a raw observer
//! resolves its keys through, read once in the namespace.

pub mod admin;
pub mod conform;
pub mod consume;
pub mod contracts;
pub mod lens;
pub mod monitor;
pub mod operation;
pub mod presence;
pub mod query;
pub mod scout;
pub mod seed;
pub mod serve;
pub mod session;
pub(crate) mod teardown;
pub mod why;
pub mod write;

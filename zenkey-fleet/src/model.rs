//! **Layer 2 — the model.** What the observations *mean*, without asking the
//! bus anything.
//!
//! One rule places a module here: *it can do its job from values already in
//! hand*. Nothing below takes a session. A key becomes a described key
//! ([`facts`]), a set of served slices becomes a queryable registry
//! ([`registry`]) and a set of rows ([`project`]), a stream of samples
//! becomes rates and latencies ([`stats`]) or a tree ([`tree`], [`skeleton`]),
//! two payloads become a diff ([`diff`]), a payload plus a schema becomes a
//! rendering ([`decode`]), a sample on the alert plane becomes an alert
//! transition ([`alert`]), the admin space's declared readers become a
//! ranked consumer list ([`consumers`]), a window of samples becomes a
//! lane-partitioned ordering on a stated clock ([`timeline`]), a sample seen
//! after a call becomes a *relation* to that call's procedure ([`trace`]).
//!
//! Being session-free is the useful property, not an accident of history: it
//! is what lets a frontend replay a `.zrec` through the same projections it
//! runs live, and what lets these modules be unit-tested without a bus. A
//! function here that finds it needs a session belongs in [`crate::bus`],
//! with this layer projecting what it brought back.
//!
//! The layer also owns the two mechanisms every long-running projection
//! shares: [`bounded`] (the O6 ceiling and its eviction ledger) and
//! [`retain`] (the retention budget). A model that grows without a bound is
//! not a model of a fleet, it is a leak.
//!
//! Nothing here judges. `stats` reports a latency that contains clock skew
//! and says so; it does not call the transport slow. Verdicts are
//! [`crate::judge`]'s.

pub mod acl;
pub mod alert;
pub mod bounded;
pub mod consumers;
pub mod diff;
pub mod examples;
pub mod facts;
pub mod impact;
pub mod jsonschema;
pub mod project;
pub mod registry;
pub mod retain;
pub mod skeleton;
pub mod stats;
pub mod storage;
pub mod timeline;
pub mod trace;
pub mod tree;

#[cfg(feature = "decode")]
pub mod decode;

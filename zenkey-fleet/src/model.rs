//! **Layer 2 — the model.** What the observations *mean*, without asking the
//! bus anything.
//!
//! One rule places a module here: *it can do its job from values already in
//! hand*. Nothing below takes a session. A key becomes a described key
//! ([`facts`]), a set of served slices becomes a queryable registry
//! ([`registry`]), a stream of samples becomes rates and latencies
//! ([`stats`]) or a tree ([`tree`], [`skeleton`]), two payloads become a diff
//! ([`diff`]), a payload plus a schema becomes a rendering ([`decode`]), a
//! sample on the alert plane becomes an alert transition ([`alert`]), a
//! window of samples becomes a lane-partitioned ordering on a stated clock
//! ([`timeline`]), a fan-in's
//! replies become snapshot rows ([`snapshot`]), two snapshots become one
//! comparison ([`snapshot_diff`]), two deployments' hosts become one
//! alignment ([`origin_map`]), and a stream of
//! described samples becomes a metrics surface whose blind spots are series
//! of their own ([`export`], [`prom`]).
//!
//! For zk2 (#612, FJ3), a presence read and its descriptors become a catalog
//! of services, interfaces, revisions and bindings ([`catalog`], which also
//! holds the contracts loaded offline), a zk2 sample plus the revision in
//! hand becomes an honest rendering ([`render`]), and two revisions become
//! the classifier's verdict on the change ([`compat`], FJ4). What a resolved
//! verb aims at — an address or a pattern, a resource, the values given, a
//! call's plan and its request's bytes, and whether a wire key is a
//! service's own (P3) — is settled here before anything is sent
//! ([`target`], FJ5). Bytes no
//! schema reaches fall to the structural ladder ([`structural`]), which
//! v1's decode seam and zk2's rendering share.
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
pub mod catalog;
pub mod compat;
pub mod diff;
pub mod examples;
pub mod export;
pub mod facts;
pub mod impact;
pub mod jsonschema;
pub mod origin_map;
pub mod prom;
pub mod registry;
pub mod render;
pub mod retain;
pub mod skeleton;
pub mod snapshot;
pub mod snapshot_diff;
pub mod stats;
pub mod storage;
pub mod structural;
pub mod target;
pub mod timeline;
pub mod tree;

#[cfg(feature = "decode")]
pub mod decode;

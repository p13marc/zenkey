//! **Layer 2 — the model.** What the observations *mean*, without asking the
//! bus anything.
//!
//! One rule places a module here: *it can do its job from values already in
//! hand*. Nothing below takes a session. A presence read and its descriptors
//! become a catalog of services, interfaces, revisions and bindings
//! ([`catalog`], which also holds the contracts loaded offline); a zk2
//! sample plus the revision in hand becomes an honest rendering
//! ([`render`]); two revisions become the classifier's verdict on the
//! change ([`compat`]). What a resolved verb aims at — an address or a
//! pattern, a resource, the values given, a call's plan and its request's
//! bytes, and whether a wire key is a service's own (P3) — is settled here
//! before anything is sent ([`target`]). A raw observer's wire key is
//! resolved rung by rung through a namespace ([`namespace`]), a presence
//! read and the contracts in hand ([`lens`]): its address and resource, its
//! payload checked against the declared type, its stamp attributed to its
//! owner. Bytes no schema reaches fall to the structural ladder
//! ([`structural`]).
//!
//! The rest is generation-neutral: a stream of samples becomes rates and
//! latencies ([`stats`]) or a tree ([`tree`]), two payloads become a diff
//! ([`diff`]), a window of samples becomes a lane-partitioned ordering on a
//! stated clock ([`timeline`]), a fan-in's replies become snapshot rows
//! ([`snapshot`]), two snapshots become one comparison by zk2 key
//! ([`snapshot_diff`]), an enrollment and the contracts become a router's
//! access control ([`acl`]), a deployment file becomes its storages
//! ([`storage`]), and a presence read and the routers' verified session
//! lists become each instance's attachment ([`attach`], #705).
//!
//! v1's half of this layer — the described key (`facts`), the registry
//! slice set (`registry`), the skeleton tree, the schema-aware decode seam
//! and the exporter — left at #612's FJ9; the `v1` branch keeps it.
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
pub mod attach;
pub mod bounded;
pub mod catalog;
pub mod compat;
pub mod diff;
pub mod examples;
pub mod jsonschema;
pub mod lens;
pub mod namespace;
pub mod render;
pub mod retain;
pub mod snapshot;
pub mod snapshot_diff;
pub mod stats;
pub mod storage;
pub mod structural;
pub mod target;
pub mod timeline;
pub mod tree;

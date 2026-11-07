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
//! ([`structural`]). One read of presence and the liveliness events after
//! it become each address's live presence — up, down or unobservable, and
//! every instance, interface and member move as a value
//! ([`presence_live`], #614); one watcher's reading of it, per address a
//! pattern matches, held or gone ([`presence_watch`], `instance-gone`'s in
//! both zenctl and zenwatch, #747). For a tree (#614, FL4): the deployment's
//! declared keyspace, from the same lens, merged with what was observed and
//! what is watched into one typed tree ([`skeleton`]), and each wire key's
//! identity resolved once, bounded and dropped when the lens's inputs move
//! ([`identity`]). A sample carrying a zk2c envelope (core §7.5) is told
//! by its key and Encoding, and its record read, its run and seq what
//! orders it, in one place every reader goes through ([`inband`]). The
//! deployment catalogs a tool is given (core §8.6) say which epoch an
//! owner's packed `time` fields count from ([`deployment`], #980 C1c). A
//! frame's records are read by a reader per key into their field states
//! ([`frame`], core §7.7). The devices a face policy lists on its device
//! faces are the owners a tool knows are on a first hop, each reported
//! link closed, never down ([`first_hop`], observer §3.11, #982 H3). A
//! raw observer's scope across a constrained face is read from the
//! contracts in hand and, given a face policy, the face's plan
//! ([`face_scope`], observer §7.5, #990 V1); with no descriptor in hand,
//! the revision of a key is the deployment's word, then CH's tag
//! ([`choice`], #990 V2a). A `.zrec` version-4 header stands in presence's
//! place for a capture read offline ([`recorded`], #990 V4).
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
//! slice set (`registry`), v1's skeleton tree, the schema-aware decode seam
//! and the exporter — left at #612's FJ9; the `v1` branch keeps it. The
//! skeleton and the facts cache came back for zk2 at FL4, as [`skeleton`]
//! and [`identity`].
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
pub mod alarms;
pub mod arbitration;
pub mod attach;
pub mod bounded;
pub mod catalog;
pub mod choice;
pub mod compat;
pub mod config;
pub mod deployment;
pub mod desired;
pub mod diff;
pub mod examples;
pub mod face_scope;
pub mod first_hop;
pub mod frame;
pub mod health;
pub mod identity;
pub mod inband;
pub mod jsonschema;
pub mod lens;
pub mod namespace;
pub mod presence_live;
pub mod presence_watch;
pub mod recorded;
pub mod redundancy;
pub mod render;
pub mod retain;
pub mod skeleton;
pub mod snapshot;
pub mod snapshot_diff;
pub mod stats;
pub mod storage;
pub mod structural;
pub mod target;
pub mod telemetry;
pub mod timeline;
pub mod timing;
pub mod tree;

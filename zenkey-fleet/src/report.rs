//! Typed reports — the shared contract between the engine and every frontend.
//!
//! Moved from zenctl (issue #34; the redesign doc called this move "one
//! refactor unlocking scripting, tests, GUI parity"). A report struct is the
//! stable output shape: zenctl renders it as a table or serde JSON/NDJSON,
//! zengui renders it as widgets, and both stay in agreement because neither
//! owns it.
//!
//! ## The placement rule
//!
//! > **Every serde-pinned wire shape in this crate lives under `report/`,
//! > split by domain, and its pinned-shape test lives next to it. A type the
//! > wire never sees stays in the module that computes it.**
//!
//! Serde is the whole test. A `#[derive(Serialize)]` (or `Deserialize`) on a
//! public type means somebody outside this process reads it: a
//! `--format json` consumer, a script, a `.zrec` file, a pane. That is a
//! contract, it changes only deliberately, and it belongs where the contracts
//! are. A type without it — `StatsTable`, `RetentionStats`, `SchemaStore`,
//! `FetchedValue` — is an implementation detail of whichever module computes
//! it, and moving it here would only put distance between it and its
//! callers.
//!
//! The rule exists because there was none. Forty shapes lived in one
//! 1,800-line `report.rs` while `WhyReport` lived in `why.rs`,
//! `Transition` and `WatchdogSummary` in `condition.rs`, `ReplayReport` and
//! `GenReport` in theirs, `LatencyReport` in `stats.rs`, `SliceDisagreement`
//! in `registry.rs`, `ProducerInfo` in `roster.rs` — and nothing said which
//! was right, so a new shape landed wherever it was first needed and the
//! split widened by one every time. The rule has no exceptions: not for a
//! shape that "belongs with its producer", not for a one-variant enum. An
//! exception is how the last rule died.
//!
//! **Domains, not layers.** The files below are named for what a shape is
//! *about* — `topic`, `doctor`, `blob`, `tape` — because that is the axis a
//! reader looking for a shape thinks along. Which layer produced it
//! ([`crate::bus`], [`crate::model`], [`crate::judge`], [`crate::tape`]) is
//! deliberately not the axis: `topic` gathers the shapes of one plane
//! whether they were observed, projected or judged, and splitting them by
//! producer would scatter one contract across four files.
//!
//! **One public namespace.** The domain files are private modules,
//! re-exported flat: every shape is spelled `zenkey_fleet::report::Thing`,
//! never `report::topic::Thing`. There is one path to each item, so the
//! split is free to be re-cut — a domain that grows can be halved, two that
//! never differed can be merged — without a single call site moving. The
//! files are organisation; the module is the interface.
//!
//! **zk2's domains** (#612, FJ3) sit beside v1's until FJ9 deletes those:
//! `presence` (services, instances and their tokens and descriptors —
//! v1's `service` is the `@rpc` plane, a different thing), `iface` (one
//! interface across a deployment — v1's `interface` is a type across
//! producers), `graph` (the binding graph, R3), `contract` (a revision's
//! view, and what asking for it found) and `payload` (a sample rendered
//! through its contract, or honestly without one). FJ5 added the acts and
//! the reads through a contract: `operation` (a call, its answer and its
//! silence — v1's `call` is the `@rpc` plane), `state` (the owner's current
//! state or an archive's last-known one, never confused) and `watch` (a
//! subscription's samples and how it ended). FJ6 gave zk2 the `doctor`
//! domain (one verdict per check, `CheckId`, `DoctorReport`); v1's check
//! vocabulary, which `field` and `check conform` still produce, moved to
//! `v1_checks` under v1 names (`V1CheckId`, `V1Finding`). Their type names do
//! not collide, so the flat namespace holds both generations. FJ8a gave the
//! mock owner its domain (`mock`: `gen`'s plan and report, `serve`'s calls)
//! in place of v1's `generate`, and re-cut `bench` over zk2 calls.

mod acl;
mod admin;
mod alert;
mod asked;
mod bench;
mod blob;
mod call;
mod catalog;
mod condition;
mod conform;
mod contract;
mod cutover;
mod diff;
mod doctor;
mod expect;
mod export;
mod field;
mod graph;
mod iface;
mod impact;
mod judgement;
mod mock;
mod node;
mod operation;
mod payload;
mod presence;
mod rate;
mod registry;
mod retired;
mod schema;
mod scout;
mod seed;
mod snapshot;
mod state;
mod storage;
mod tape;
mod timeline;
mod topic;
mod v1_checks;
mod watch;
mod why;

pub use acl::*;
pub use admin::*;
pub use alert::*;
pub use asked::*;
pub use bench::*;
pub use blob::*;
pub use call::*;
pub use catalog::*;
pub use condition::*;
pub use conform::*;
pub use contract::*;
pub use cutover::*;
pub use diff::*;
pub use doctor::*;
pub use expect::*;
pub use export::*;
pub use field::*;
pub use graph::*;
pub use iface::*;
pub use impact::*;
pub use judgement::*;
pub use mock::*;
pub use node::*;
pub use operation::*;
pub use payload::*;
pub use presence::*;
pub use rate::*;
pub use registry::*;
pub use retired::*;
pub use schema::*;
pub use scout::*;
pub use seed::*;
pub use snapshot::*;
pub use state::*;
pub use storage::*;
pub use tape::*;
pub use timeline::*;
pub use topic::*;
pub use v1_checks::*;
pub use watch::*;
pub use why::*;

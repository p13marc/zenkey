//! zk2 keys and contracts, without a Zenoh session.
//!
//! This is the static half of zk2 (epic #585, issue #608): everything that
//! can be decided from files, and never needs a bus. The design of record is
//! `docs/zk2/architecture.md` (r4), stated normatively by `spec/core.md`;
//! the authoring format is draft 3
//! (`examples/zk2/README.md`).
//!
//! Layer map:
//! - [`chunk`]: the plain-chunk charset and the small lexical predicates.
//! - [`slug`]: canonical, injective slugging of foreign values into chunks,
//!   ported unchanged from v1.
//! - [`grammar`]: the zk2 key grammar. Data keys, instance and interface
//!   tokens, member tokens and contract keys, as typed values over
//!   `zenoh_keyexpr::OwnedKeyExpr`, with a parser that refuses anything
//!   outside the grammar; and §1.3's selectors: the ambient one, the exact
//!   kind selectors, and the verbatim chunks a selector cannot reach
//!   ([`grammar::blind_spots`], #614 FL4).
//! - [`template`]: resource templates, with build, match, overlap and the
//!   most-literal-first precedence (r3.3 D1).
//! - [`authoring`]: the TOML authoring format, draft 3, as serde types;
//!   `spec/contract.schema.json` is generated from them.
//! - [`diag`]: diagnostics with stable codes, the conformance surface of
//!   the lints.
//! - [`schema`]: schema artifacts (protobuf through `protox`, JSON Schema)
//!   and type resolution.
//! - [`contract`]: the contract model: defaults expanded, types resolved,
//!   every lint run; or read back from a bundle
//!   ([`contract::Contract::from_bundle`], #611).
//! - [`canonical`]: the canonical JSON form, its restrictions, and the
//!   fingerprint.
//! - [`bundle`]: the bundle container, built and verified Merkle-style.
//! - [`history`]: the append-only `.history` of published revisions.
//! - [`compat`]: the FULL_TRANSITIVE compatibility classifier (spec §9.8),
//!   over revisions from contracts or bundles, and the retention rule's
//!   identity check.
//! - [`budget`]: the population budget (core §2.7, 0.24; #735): a
//!   templated resource's bound in an instance, what an event's `rate`
//!   bounds, and the pure judgement of a reading against them. The reads
//!   are the tools'.
//! - [`catalog`]: the deployment catalog (core §8.6; #980 C1a): its
//!   reading with §8.6's refusals, its id, append-only succession, and a
//!   reader's resolution of the catalog an owner names, through a
//!   successor below its predecessor's count. It holds the deployment
//!   epoch and each revision's index; provisioning it is the deployment's.
//! - [`descriptor`]: the descriptor record an instance serves, checked
//!   against its contracts; `spec/descriptor.schema.json` is generated
//!   from it.
//! - [`envelope`]: the error envelope of a failed call, decoded and
//!   checked by its Zenoh encoding (`spec/core/error.proto`).
//! - [`decode`]: a payload decoded from a bundle alone, for tools (§7.2).
//! - [`validate`]: a value checked against a JSON Schema type of a bundle,
//!   the §7.3 subset and nothing more, for tools that build a payload from
//!   JSON (#671).
//! - [`hostid`]: `hostid.v1`'s derivation of a system from a machine id
//!   (`spec/profiles/hostid/v1.md`, #719). The inputs, the shared file and
//!   failing closed are the runtime's.
//! - [`freshness`]: `freshness.v1`'s horizon and its pure judgement of a
//!   member's freshness (`spec/profiles/freshness/v1.md`, #720). The
//!   owner's re-puts are the runtime's.
//! - [`seed`]: a late joiner's merge (core §4.2, 0.26; 0.27, #752): per
//!   key, which arriving sample replaces what is held, by the stamp — an
//!   unstamped sample ranked below every stamped one, equal times ordered
//!   by the id as zenoh orders it. The GET and the subscription are the
//!   tools'.
//! - [`health`]: `health.v1`'s levels, fault codes, and its pure judgement
//!   of one reading of a service, on top of [`freshness`]
//!   (`spec/profiles/health/v1.md`, #721). The owner's puts and its
//!   `clock_ahead` fault are the runtime's.
//! - [`config`]: `config.v1`'s wire types, its validators, the window,
//!   redaction, and a pure state machine of one owner's configuration
//!   (`spec/profiles/config/v1.md`, #768). The device, the persisted layer
//!   and the timers are the runtime's.
//! - [`link`]: `link.v1`'s exposure, the face policy a deployment writes
//!   and its lints, the plan a generator derives from both (denies, one
//!   downsampling rule per concrete key, the attachment, window floors,
//!   liveness evidence), and the checks of a router's block
//!   (`spec/profiles/link/v1.md`, #769). The router's configuration is the
//!   generator's.
//! - [`alarms`]: `alarms.v1`'s key recipe, byte for byte ZenSight's, with
//!   its sanitizing step; the document and its checks; one key's samples
//!   read into transitions; a reference to an alarm; and §5's questions,
//!   on top of [`freshness`] (`spec/profiles/alarms/v1.md`, #800). The
//!   owner's puts, its confirmations and its adoption at a restart are the
//!   runtime's.
//! - [`telemetry`]: `telemetry.v1`'s vocabulary and standard schema, the
//!   declaration read from a resolved resource, one sample's conformance
//!   with the member's kind and unit, and a pair's continuity and rate, its
//!   epoch in-band or from presence (`spec/profiles/telemetry/v1.md`,
//!   #819). The owner's in-band epoch is the runtime's.
//! - [`desired`]: `desired.v1`'s vocabulary and standard schema, the
//!   declaration read from a resolved resource, a requirement's binding and
//!   its slice's key, the standard `Report` with its reading and its
//!   consistency, a target's pure two-phase state machine (each version
//!   ordered by its author's stamp, acted on once, the same document
//!   adopted without an act, refused whole or reverted), and §5's four
//!   questions (`spec/profiles/desired/v1.md`,
//!   #840). The author's writes and record, and the target's subscription,
//!   seed and archive read, are the runtime's.
//! - [`redundancy`]: `redundancy.v1`'s vocabulary, the declaration read
//!   from a contract (governed, its election `claim` or `external`), how
//!   instance ids compare, the descriptor's standby mark read and checked,
//!   the election's decision over one read and the heal's over two, one
//!   candidate's pure two-phase state machine (claim, stand by, take over,
//!   step down), and §5's four questions of an address and an instance
//!   (`spec/profiles/redundancy/v1.md`, #841). The reads, the watch, the
//!   timers, the lock and the acts in core §8.2's order are the runtime's.
//! - [`timing`]: `timing.v1`'s vocabulary, the declaration read from a
//!   resolved resource (a stream's period, deadline and lifespan), a
//!   reader's deadline tracker for one provider and resource as a pure state
//!   machine (missed, met and ended, edge-triggered, its detection bound,
//!   the members it holds under the bound), one sample's lifespan on the
//!   deployment's stated bound, the period over a tool's window, and §5's
//!   three questions (`spec/profiles/timing/v1.md`, #908). The subscription,
//!   the timer, the presence it follows and the stamps it reads are the
//!   runtime's.
//! - [`arbitration`]: `arbitration.v1`'s vocabulary, a role's declaration
//!   read from its contract's requirement and the revision of the required
//!   interface held (the first profile to read a requirement's
//!   annotations), the ranking of a binding's providers, one role's
//!   arbitration as a pure state machine (eligibility by the deadline and
//!   by withdrawals, re-admission by a token or a read, a move of the watch
//!   held back, the active provider and its switches, the dead-man), and
//!   §5's three questions (`spec/profiles/arbitration/v1.md`, #909). The
//!   subscription, the watch, the reads, the timer and the safe value are
//!   the runtime's and the application's.
//! - [`packed`]: the `packed` type kind (core §7.6, 0.75; #979 B1): its
//!   schema file and lints (E038 to E045), each type's layout, its layout
//!   form and tag, and an interpreted codec over JSON values, exact in its
//!   fixed point: the source of truth generated code (B2) matches.
//! - [`frame`]: frames (core §7.7, 0.83; #981 K1): a frame resource's
//!   `frame` table resolved (E048), its layout over a packed record type
//!   with no marker, the frame form and tag, and one body read and written
//!   back, with §7.7's refusals. A reader's field states across frames are
//!   K2's, the owner's writer K3's.
//! - [`wire`]: the zk2c envelope (core §7.5, 0.72; #976): CH's form, run
//!   width and tag, the run and the seq, a put's mutation and a delete's
//!   attachment, parsed without a contract with §7.5's refusals and written
//!   back byte for byte. Defined ahead of its use: no rule adopts it yet,
//!   and nothing else in the workspace reads or writes one.
//! - [`heartbeat`]: the zk2c envelope's form 6 (core §7.5, 0.81; #978's
//!   F1): a revision's collections (its `state` and `@state` resources with
//!   a horizon above 0) as a [`heartbeat::Layout`] with its tag, a body
//!   read and written with them and §7.5's refusals, and a collection's
//!   [`heartbeat::Digest`]. `freshness.v2` adopts it (core 0.82; #978's
//!   F3): [`freshness::uses_v2`], and a reader's [`freshness::View`].
//!
//! No item here opens a session or depends on `zenoh`: build scripts, CI
//! tools and the Python verifier's fixtures all stand on this crate.

pub mod alarms;
pub mod arbitration;
pub mod authoring;
pub mod budget;
pub mod bundle;
pub mod canonical;
pub mod catalog;
pub mod chunk;
pub mod compat;
pub mod config;
pub mod contract;
pub mod decode;
pub mod descriptor;
pub mod desired;
pub mod diag;
pub mod envelope;
pub mod frame;
pub mod freshness;
pub mod grammar;
pub mod health;
pub mod heartbeat;
pub mod history;
pub mod hostid;
pub mod link;
pub mod packed;
pub mod redundancy;
pub mod schema;
pub mod seed;
pub mod slug;
mod strict;
pub mod telemetry;
pub mod template;
pub mod timing;
mod toml10;
mod unbundle;
pub mod validate;
pub mod wire;

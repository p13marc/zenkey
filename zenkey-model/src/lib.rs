//! zk2 keys and contracts, without a Zenoh session.
//!
//! This is the static half of zk2 (epic #585, issue #608): everything that
//! can be decided from files, and never needs a bus. The design of record is
//! `docs/zk2/architecture.md` (r4), stated normatively by `spec/core.md`;
//! the authoring format is draft 1
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
//! - [`authoring`]: the TOML authoring format, draft 1, as serde types;
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
//!
//! No item here opens a session or depends on `zenoh`: build scripts, CI
//! tools and the Python verifier's fixtures all stand on this crate.

pub mod alarms;
pub mod authoring;
pub mod budget;
pub mod bundle;
pub mod canonical;
pub mod chunk;
pub mod compat;
pub mod config;
pub mod contract;
pub mod decode;
pub mod descriptor;
pub mod diag;
pub mod envelope;
pub mod freshness;
pub mod grammar;
pub mod health;
pub mod history;
pub mod hostid;
pub mod link;
pub mod schema;
pub mod seed;
pub mod slug;
mod strict;
pub mod template;
mod toml10;
mod unbundle;
pub mod validate;

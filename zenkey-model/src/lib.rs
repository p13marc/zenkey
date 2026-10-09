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
//!   outside the grammar.
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
//! - [`descriptor`]: the descriptor record an instance serves, checked
//!   against its contracts; `spec/descriptor.schema.json` is generated
//!   from it.
//! - [`envelope`]: the error envelope of a failed call, decoded and
//!   checked by its Zenoh encoding (`spec/core/error.proto`).
//! - [`decode`]: a payload decoded from a bundle alone, for tools (§7.2).
//! - [`validate`]: a value checked against a JSON Schema type of a bundle,
//!   the §7.3 subset and nothing more, for tools that build a payload from
//!   JSON (#671).
//!
//! No item here opens a session or depends on `zenoh`: build scripts, CI
//! tools and the Python verifier's fixtures all stand on this crate.

pub mod authoring;
pub mod bundle;
pub mod canonical;
pub mod chunk;
pub mod compat;
pub mod contract;
pub mod decode;
pub mod descriptor;
pub mod diag;
pub mod envelope;
pub mod grammar;
pub mod history;
pub mod schema;
pub mod slug;
mod strict;
pub mod template;
mod toml10;
mod unbundle;
pub mod validate;

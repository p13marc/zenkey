//! zk2 keys and contracts, without a Zenoh session.
//!
//! This is the static half of zk2 (epic #585, issue #608): everything that
//! can be decided from files, and never needs a bus. The design of record is
//! `docs/zk2/architecture.md` (r3.3); the authoring format is draft 1
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
//!   every lint run.
//! - [`canonical`]: the canonical JSON form, its restrictions, and the
//!   fingerprint.
//! - [`bundle`]: the bundle container, built and verified Merkle-style.
//! - [`history`]: the append-only `.history` of published revisions.
//!
//! No item here opens a session or depends on `zenoh`: build scripts, CI
//! tools and the Python verifier's fixtures all stand on this crate.

pub mod authoring;
pub mod bundle;
pub mod canonical;
pub mod chunk;
pub mod contract;
pub mod diag;
pub mod grammar;
pub mod history;
pub mod schema;
pub mod slug;
mod strict;
pub mod template;

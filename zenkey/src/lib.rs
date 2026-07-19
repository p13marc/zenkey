//! Executable form of the keyspace-v2 convention.
//!
//! The convention is specified in `rfcs/` (v1). This crate is
//! its enforcement layer: everything a producer or consumer needs to emit and
//! parse conforming keys without ever spelling a raw key string.
//!
//! Canonical grammar (base-relative — the deployment base is the session
//! *namespace*, RFC 03 §1.1, so no key built here contains it):
//!
//! ```text
//! v1/<origin>/<class>/<producer>/<subject...>
//! ```
//!
//! Layer map:
//! - [`key`] — [`Key`]/[`Selector`]/[`Chunk`]: validated key value types over
//!   `zenoh_keyexpr::OwnedKeyExpr` (RFC 08 §1.2).
//! - [`grammar`] — chunk lexical rules, reserved tokens, structural key
//!   assembly and parsing (RFC 03).
//! - [`origin`] — `h-<12hex>` host-origin minting (RFC 06 §1).
//! - [`profile`] — the application profile: app name + origin salt, the two
//!   constants an adopting application declares (RFC 06 §1, RFC 11 §4).
//! - [`slug`] — canonical, injective slugging of foreign values (RFC 03 §2).
//! - [`qos`] — the five named QoS profiles (RFC 04 §3).
//! - [`context`] — [`V1Context`]: origin + producer; producers build all
//!   framework keys through it.
//! - [`slice`] — [`RegistrySlice`], the `introspect` reply type + diff
//!   (RFC 08 §6).
//!
//! The subject vocabulary itself is governed by the registry (RFC 08). It is
//! **application-owned**: each application checks its `registry/*.toml` into
//! its own repository and generates typed subject builders/parsers from them
//! with the `zenkey-build` crate in its build script. This crate ships no
//! registry.
//!
//! The RFC's design properties D1–D6 are pinned as executable guard tests in
//! `tests/guard.rs` — run by CI, as RFC 03 §4 requires.
//!
//! # Note on the deployment base
//!
//! There is deliberately no base constant in this crate. The base is the value
//! a deployment sets as its Zenoh session **`namespace`**, which prefixes it
//! onto every keyexpr the session emits, strips it on delivery, and *filters*
//! ingress from outside it — an isolation boundary, not a string convention
//! (RFC 09 §0). The only legitimate readers are session configuration,
//! router-side artifacts (storage selectors, ACL rules), and deliberately
//! un-namespaced debug tools (`zenctl`). Application code that reaches for a
//! base to *build a key* has made a mistake: the session adds the base.

pub mod common_state;
pub mod context;
pub mod grammar;
pub mod key;
pub mod origin;
pub mod pattern;
pub mod profile;
pub mod qos;
pub mod slice;
pub mod slug;

pub use common_state::CommonState;
pub use context::V1Context;
pub use grammar::{
    Class, ClassOrPlane, KeyError, Origin, Plane, Producer, StructuralKey, VERSION_CHUNK,
};
pub use key::{Chunk, Key, Selector};
pub use origin::{ConcreteOrigin, Fleet, HostId, LocalOrigin, RemoteOrigin, ServiceOrigin};
pub use profile::AppProfile;
pub use qos::QosProfile;
pub use slice::{RegistrySlice, SliceFinding, parse_slice};

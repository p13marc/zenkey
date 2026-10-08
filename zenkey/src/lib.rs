//! The zk2 runtime: the session half of zk2 (#610 onward).
//!
//! Everything that needs no session (keys, contracts, bundles, the
//! descriptor check, the error envelope) is `zenkey-model`'s, re-exported
//! here as [`model`]. This crate adds what a Zenoh session does with them,
//! rule by rule from the spec (`spec/core.md`):
//!
//! | Module | Spec |
//! |---|---|
//! | [`config`]: what a deployment configures | §1.5, §3.2 R1–R2, §8.1 (the tokenless set) |
//! | [`service`]: bring-up, the descriptor, tokens, epochs, members | §1.5, §3.3, §8.1–§8.2 |
//! | [`presence`]: reading tokens and descriptors | §8.1 |
//! | [`retrieval`]: fetching a contract bundle | §8.4 |
//! | [`writer`]: the contract's QoS and `Encoding` on every sample; events | §2.4–§2.6, §7.2 |
//! | [`consumer`]: data through a role's bindings | §3.2 R1–R7 |
//! | [`shm`]: the memlock limit SHM falls back under | §7.4 |
//!
//! **Zenoh stays reachable.** The runtime never owns the session: a
//! service holds a clone, every key is available as a key expression, and
//! raw resources are plain zenoh publishers and queryables declared with the
//! contract's QoS. The typed layers (streams #619, state #620, operations
//! #621) are built on these.

pub mod config;
pub mod consumer;
mod descriptor;
pub mod error;
pub mod implementation;
pub mod presence;
mod qos;
pub mod retrieval;
pub mod service;
pub mod shm;
pub mod writer;

pub use zenkey_model as model;

pub use config::{Binding, ServiceConfig};
pub use error::{Error, Result};
pub use implementation::Implementation;
pub use service::{Service, ServiceBuilder};

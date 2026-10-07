//! zk2rt: a throwaway zk2 runtime over `zenkey-model`, for the spike only
//! (#591, r3 §7). It is not the runtime (#610, #619–#621): it is just enough
//! of r3 to measure its claims, and it is never merged.

pub mod client;
pub mod config;
pub mod metrics;
pub mod mock;
pub mod service;

/// The zenoh release every number is measured against (the workspace pin).
pub const ZENOH_VERSION: &str = "1.10.1";

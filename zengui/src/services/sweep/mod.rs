//! Asking the fleet a whole question at once (#175).
//!
//! What separates these from [`super::value`] is not the transport — it is all
//! `fleet_get` underneath — but what an answer is *about*: a sweep's answer
//! describes a deployment, so every one of them carries the base it ran
//! against back to the pane. That is the staleness guard's other half (#109),
//! and it is the reason so many of these return a tuple rather than a plain
//! `Result`: the pane cannot tell whether an answer is still true without
//! knowing what it was asked of.
//!
//! One module per question (#614, FL5), so the chunks that port them in
//! parallel do not meet in one file. Everything is re-exported flat: a
//! call site still says `services::sweep::<fn>`.

mod admin;
mod blob;
mod budget;
mod consumers;
mod doctor;
mod nodes;
mod registry;

pub use admin::admin;
pub use blob::{BlobFetch, blob_fetch, blob_probe, blob_tree};
pub use budget::budget;
pub use consumers::consumers_of;
pub use doctor::doctor;
pub use nodes::node_info;
pub use registry::skeleton;

//! `Render` for the report types (#198).
//!
//! One file per family group. Each impl is a straight port of the renderer it
//! replaces in `output.rs`, and the port is where three things stop being
//! prose and become structure:
//!
//! * the empty-result sentence that used to `return` early out of the table
//!   arm becomes a [`Note`](super::Note), so it reaches json and ndjson too —
//!   `zenctl topic list --format ndjson` against an empty fleet printed
//!   nothing at all;
//! * `—` for *not asked* and `""` for *asked, nothing there* become different
//!   [`Cell`](super::Cell) variants rather than two spellings a refactor can
//!   merge by accident (RFC 09 §5.1 O4);
//! * a heterogeneous row stream gets a discriminator, because a consumer
//!   should not have to identify a line by guessing at its fields.

mod bench;
mod blobs;
mod calls;
mod closing;
mod documents;
mod findings;
mod observations;
mod rate;
pub use rate::RateView;
mod listings;
pub mod local;
mod services;

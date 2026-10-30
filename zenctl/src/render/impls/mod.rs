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
//!   merge by accident (observer §1);
//! * a heterogeneous row stream gets a discriminator, because a consumer
//!   should not have to identify a line by guessing at its fields.

mod acl;
pub mod acts;
mod alarms;
mod arbitration;
mod bench;
mod closing;
mod config;
mod desired;
mod fields;
mod findings;
pub mod observations;
mod rate;
mod redundancy;
pub use rate::RateView;
mod listings;
pub mod local;
mod mock;
mod snapshot;
pub mod storage;
mod telemetry;
mod timeline;
mod timing;
pub mod zk2;

/// What a reader's selector cannot reach, as notes (`spec/observer.md`
/// §8.2; zk2py round 82's F-287, #950): the verbatim chunks it names none
/// of, said as the raw observers' notes say them (#924), and the resources
/// of `kind` of the owners in scope that it misses. `timing` and
/// `telemetry` share it; nothing when the report holds no selector.
fn reach_notes(reach: Option<&zenkey_fleet::report::ScopeReach>, kind: &str) -> Vec<super::Note> {
    let Some(reach) = reach else {
        return Vec::new();
    };
    let mut notes = Vec::new();
    if !reach.excluded.is_empty() {
        let named = crate::cmd::zk2::named_verbatim(&reach.excluded);
        notes.push(
            super::Note::coverage(format!(
                "{}{} excluded from this reading, not empty: the selector names none of them, \
                 and `*`/`**` never match a verbatim chunk",
                if named.is_empty() {
                    String::new()
                } else {
                    format!("{} named by the selector; ", named.join(", "))
                },
                reach.excluded.join(", ")
            ))
            .cite("observer §8.2"),
        );
    }
    let missed = reach.unreached_words(kind);
    if !missed.is_empty() {
        notes.push(super::Note::caveat(missed).cite("observer §8.2"));
    }
    notes
}

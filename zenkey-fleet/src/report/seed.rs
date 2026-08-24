//! The seeding plane: what each path of a seeded subscription contributed.
//!
//! Every count here reads three ways on purpose — `None` is a path that was
//! switched off, `Some(0)` is a path that ran and nobody answered, and a
//! number is a number (RFC 09 §5.1 O4).

/// What each seed path contributed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct SeedCoverage {
    /// Replies from the `@adv` cache query (`None` = the history path was
    /// disabled; `Some(0)` = ran and no cache answered — an observation,
    /// not a verdict).
    pub history_replies: Option<usize>,
    /// Replies from the storage GET (same `None`/`Some(0)` reading).
    pub storage_replies: Option<usize>,
    /// Samples suppressed by the merge — not newer than what was already
    /// seen for their key. The honesty counter: a seed that arrived late
    /// and lost (or duplicated the other path) is counted, never silently
    /// absorbed.
    pub superseded: u64,
}

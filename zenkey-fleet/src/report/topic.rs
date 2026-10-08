//! The topic plane: the vocabulary v1's key reports still share — how far
//! the description ladder reached ([`TopicVerdict`], read by snapshots and
//! the admin sweep) and what a cardinality window rests on
//! ([`BudgetWindow`]). The topic listings and `topic info` left with v1's
//! `topic` noun at FJ4 (#612).

use serde::Serialize;

/// What a `--budget` column's numbers rest on (#221) — the O5/O6 coverage
/// statement: the window, the exact scopes watched, and the observer's
/// bound. Without it "observed 3" reads as "the population is 3", which a
/// bounded sweep never established.
#[derive(Debug, Clone, Serialize)]
pub struct BudgetWindow {
    pub window_s: f64,
    /// The selectors actually watched — coverage is a statement, not a vibe.
    pub scopes: Vec<String>,
    /// Distinct keys the bounded observer retained.
    pub keys: usize,
    /// Keys the observer retired to stay within its bound; non-zero makes
    /// every observed count a lower bound twice over.
    pub evicted: u64,
}

/// Where the ladder stopped. Serialized snake_case; stable for scripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TopicVerdict {
    /// Parses, refines, declared — the full story is present.
    Registered,
    /// Parses as a v1 data key; the producer's slice does not declare it.
    Unregistered,
    /// Parses; no loaded slice covers this producer (or service origin).
    NoSliceForProducer,
    /// Parses, but onto a verbatim plane — there is no `[[subject]]` surface
    /// to consult (RFC 03 §1.4).
    NotADataClass,
    /// A legal Zenoh key that is not this convention's (O1: a fact).
    NotV1,
    /// Sits under a different deployment base than the one configured.
    NotUnderBase,
    /// Parses as a data key, but no registry has been loaded — "not asked"
    /// is not "answered no" (O4).
    RegistryNotLoaded,
}

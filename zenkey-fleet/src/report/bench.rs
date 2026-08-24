//! The bench plane: a measured round-trip, per origin and in aggregate.

use serde::Serialize;

/// One origin's reply-latency distribution in a benchmark (issue #52).
/// Timed **per reply**, so a fast origin in a fan-out is not charged the
/// slowest origin's round trip.
#[derive(Debug, Clone, Serialize)]
pub struct OriginLatency {
    pub origin: String,
    pub replies: usize,
    pub min_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

/// `zenctl bench rpc` (issue #52).
#[derive(Debug, Clone, Serialize)]
pub struct BenchReport {
    pub key: String,
    pub requested: usize,
    pub completed: usize,
    pub concurrency: usize,
    /// Error replies (RFC 05 §3) plus calls the GET itself failed.
    pub errors: usize,
    /// Calls that drew **zero** replies — counted apart from errors, because
    /// silence is not a failure and averaging it away would hide it
    /// (RFC 05 §3.1).
    pub silent: usize,
    /// Calls whose task **panicked**: a third population, apart from both
    /// `errors` and `silent` (#329).
    ///
    /// A panicked call answered nothing and measured nothing, and it is not
    /// news about the fleet — it is news about this tool. Folding it into
    /// `errors` would blame the producer for a bug here; folding it into
    /// `silent` would claim an observation nobody made. Dropping it, which is
    /// what `let Ok(result) = handle.await else { continue }` did, deletes a
    /// whole population from a report whose other counters exist precisely so
    /// that a benchmark cannot average a non-answer away (RFC 13 §3 O6).
    pub panicked: usize,
    pub elapsed_s: f64,
    pub calls_per_s: f64,
    pub origins: Vec<OriginLatency>,
}

//! The bound every long-lived table in this crate shares (RFC 09 §5.1 O6).
//!
//! An explorer that runs for hours accumulates: per-key statistics, key
//! projections, watch state. Every one of those tables needs the same three
//! things — a ceiling, an eviction that amortises, and a *count* of what the
//! ceiling cost — and two of them ([`StatsTable`](crate::model::stats::StatsTable)
//! and [`FactsCache`](crate::model::facts::FactsCache)) had shipped a
//! byte-identical copy of the mechanism: same evict fraction, same batch
//! scan, same `len - target` batch, differing only in the recency *type*.
//!
//! So the mechanism lives here, once, and the policy stays with each holder:
//! `BoundedLru::admit` returns how many entries it dropped, and the caller
//! adds that to its own ledger. "Evicted under the bound", "retired because
//! nothing watches it any more" and "never projected in the first place" are
//! different facts, and one counter over several of them is exactly what O6
//! forbids.
//!
//! Recency is the caller's too: `StatsTable` orders by an injected
//! `last_seen: Instant`, `FactsCache` by a monotone observation counter, and
//! `BoundedLru::admit` takes whichever as a projection out of the value.

use std::collections::HashMap;

/// Default key bound. Large enough that no ordinary fleet reaches it — the
/// reference application's whole telemetry fan is a few thousand keys — and
/// small enough that a runaway key family cannot exhaust memory.
pub const DEFAULT_MAX_KEYS: usize = 50_000;

/// Fraction of the table dropped when the bound is hit.
///
/// Evicting in batches amortises the O(n) scan for the oldest entries across
/// many inserts; evicting one key per insert would make every sample past the
/// bound a full table scan.
const EVICT_FRACTION: usize = 16;

/// A map bounded at `max_keys` entries, evicting the least-recently-seen in
/// batches — the mechanism behind both this module's [`StatsTable`] and
/// [`FactsCache`](crate::model::facts::FactsCache), which carried a byte-identical
/// copy of it (deep review: same [`EVICT_FRACTION`], same batch scan, same
/// `len - target` batch; only the recency *type* differed).
///
/// It owns the bound and the eviction, and deliberately **not** the ledger:
/// [`admit`](Self::admit) returns how many entries it dropped and each holder
/// adds that to its own counters. "Evicted under the bound", "retired because
/// nothing watches it any more" and "never projected in the first place" are
/// different facts, and one counter over several of them is exactly what
/// RFC 09 §5.1 O6 forbids.
///
/// Recency is the caller's too: `StatsTable` orders by the injected
/// `last_seen: Instant`, `FactsCache` by a monotone observation counter, and
/// [`admit`](Self::admit) takes whichever as a projection out of the value.
///
/// It lives here rather than in a module of its own because this is where the
/// bound was first argued — [`DEFAULT_MAX_KEYS`], [`EVICT_FRACTION`] and the
/// amortisation note `facts.rs` cites verbatim are all in this file.
#[derive(Debug)]
pub(crate) struct BoundedLru<K, V> {
    entries: HashMap<K, V>,
    max_keys: usize,
}

impl<K: std::hash::Hash + Eq + Clone, V> BoundedLru<K, V> {
    /// A map bounded at `max_keys` entries; zero is clamped to one rather than
    /// accepted, so eviction always has somewhere to stop.
    pub(crate) fn with_capacity(max_keys: usize) -> Self {
        BoundedLru {
            entries: HashMap::new(),
            max_keys: max_keys.max(1),
        }
    }

    /// The bound in force.
    pub(crate) fn max_keys(&self) -> usize {
        self.max_keys
    }

    /// Make room for one further key: if the bound is already reached, drop
    /// the least-recently-seen batch, ordering by `recency`. Returns how many
    /// entries were dropped — zero on the ordinary path — which the caller
    /// adds to its own ledger.
    ///
    /// Evicting in batches amortises the O(n) scan across many inserts;
    /// evicting one key per insert would make every insert past the bound a
    /// full scan.
    pub(crate) fn admit<R, F>(&mut self, mut recency: F) -> usize
    where
        R: Ord,
        F: FnMut(&V) -> R,
    {
        if self.entries.len() < self.max_keys {
            return 0;
        }
        let target = self.max_keys - (self.max_keys / EVICT_FRACTION).max(1);
        let mut seen: Vec<(R, K)> = self
            .entries
            .iter()
            .map(|(k, v)| (recency(v), k.clone()))
            .collect();
        // Oldest first.
        seen.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
        let doomed = self.entries.len() - target;
        let mut dropped = 0;
        for (_, key) in seen.into_iter().take(doomed) {
            if self.entries.remove(&key).is_some() {
                dropped += 1;
            }
        }
        dropped
    }

    pub(crate) fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.entries.insert(key, value)
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn keys(&self) -> impl Iterator<Item = &K> {
        self.entries.keys()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries.iter()
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = &V> {
        self.entries.values()
    }

    pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.entries.values_mut()
    }

    /// Borrowed lookup: `&str` against `String` keys, no per-sample
    /// allocation on the hot hit path (module header).
    pub(crate) fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: std::borrow::Borrow<Q>,
        Q: std::hash::Hash + Eq + ?Sized,
    {
        self.entries.get(key)
    }

    pub(crate) fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: std::borrow::Borrow<Q>,
        Q: std::hash::Hash + Eq + ?Sized,
    {
        self.entries.get_mut(key)
    }

    pub(crate) fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: std::borrow::Borrow<Q>,
        Q: std::hash::Hash + Eq + ?Sized,
    {
        self.entries.remove(key)
    }
}

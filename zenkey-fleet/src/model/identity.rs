//! Each wire key's identity, resolved once and kept (#614, FL4): the
//! successor of v1's facts cache, over the zk2 [`Lens`].
//!
//! A frontend draws the identity of every key it shows — its address and
//! resource, or the rung where resolution stopped — on every frame, and
//! [`Lens::resolve`] parses the key, walks the catalog and matches
//! templates each time it is asked. An [`IdentityCache`] asks once per key
//! and hands back the [`Resolution`] after that.
//!
//! **Bounded, and the bound says what it cost** (O6): at most `max_keys`
//! resolutions, the least recently ensured evicted in batches
//! ([`crate::model::bounded`]), and counted ([`IdentityCache::evicted`]).
//! A cache that stopped growing and a bus that went quiet look alike from
//! outside, so the ledger is public: `inserted == len + evicted +
//! invalidated`.
//!
//! **Invalidated by a generation.** A resolution is only as good as the
//! lens it was made with: a key that resolved to "no provider" resolves to
//! its resource once the provider appears, and one that resolved through a
//! revision resolves differently once a second revision is named. The lens
//! carries the generation of its inputs ([`Lens::at_generation`], e.g.
//! [`crate::LensFeed::generation`]), and the cache drops everything it
//! resolved under another one the first time it is asked under the new one
//! ([`IdentityCache::invalidated`] counts them, apart from the bound's
//! evictions: different facts, different counters).
//!
//! Session-free: a `.zrec` replay resolves through an offline lens at a
//! generation that never moves, and the cache holds exactly what a live
//! one would for the same keys.

use crate::model::bounded::{BoundedLru, DEFAULT_MAX_KEYS};
use crate::model::lens::{Lens, Resolution};

#[derive(Debug)]
struct Entry {
    resolution: Resolution,
    /// Recency for the bound: the observation counter at the last
    /// [`IdentityCache::ensure`].
    seen: u64,
}

/// Wire keys' resolutions, bounded and generation-tagged: see the module
/// doc.
#[derive(Debug)]
pub struct IdentityCache {
    entries: BoundedLru<String, Entry>,
    generation: u64,
    inserted: u64,
    evicted: u64,
    invalidated: u64,
    seq: u64,
}

impl Default for IdentityCache {
    fn default() -> Self {
        IdentityCache::with_capacity(DEFAULT_MAX_KEYS)
    }
}

impl IdentityCache {
    /// A cache bounded at `max_keys` resolutions (zero is clamped to one).
    /// Pass the bound the monitor's statistics table runs under: the cache
    /// cannot usefully outgrow the table whose keys it names.
    pub fn with_capacity(max_keys: usize) -> IdentityCache {
        IdentityCache {
            entries: BoundedLru::with_capacity(max_keys),
            generation: 0,
            inserted: 0,
            evicted: 0,
            invalidated: 0,
            seq: 0,
        }
    }

    /// `wire`'s resolution through `lens`: resolved now if it is not held
    /// under the lens's generation, and its recency bumped either way. The
    /// single insert point — the whole bound rests on that being true.
    pub fn ensure(&mut self, lens: &Lens<'_>, wire: &str) -> &Resolution {
        if lens.generation() != self.generation {
            self.invalidated += self.entries.len() as u64;
            self.entries.clear();
            self.generation = lens.generation();
        }
        self.seq += 1;
        let seq = self.seq;
        if self.entries.get(wire).is_none() {
            self.evicted += self.entries.admit(|e| e.seen) as u64;
            self.entries.insert(
                wire.to_owned(),
                Entry {
                    resolution: lens.resolve(wire),
                    seen: seq,
                },
            );
            self.inserted += 1;
        }
        let entry = self.entries.get_mut(wire).expect("held or just inserted");
        entry.seen = seq;
        &entry.resolution
    }

    /// A held resolution. Pure — recency is not touched, so it is safe from
    /// a `&self` render path — and it answers whatever generation the entry
    /// was resolved under: a frontend that wants the current one calls
    /// [`IdentityCache::ensure`] with the current lens.
    pub fn get(&self, wire: &str) -> Option<&Resolution> {
        self.entries.get(wire).map(|e| &e.resolution)
    }

    /// Every wire key held, in no particular order (#614, FL7): what a
    /// frontend lists as "observed so far" — a jump-to list, or the keys it
    /// re-resolves when its lens moves. Pure, like [`IdentityCache::get`]:
    /// listing touches no recency.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// The generation the held resolutions were made under.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The bound in force.
    pub fn max_keys(&self) -> usize {
        self.entries.max_keys()
    }

    /// Resolutions dropped to stay within the bound (O6).
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    /// Resolutions dropped because the lens moved to another generation.
    pub fn invalidated(&self) -> u64 {
        self.invalidated
    }

    /// Resolutions *made* since the last [`IdentityCache::clear`]: the other
    /// half of the ledger, `inserted == len + evicted + invalidated`. A key
    /// evicted and ensured again is resolved again, which is the cost the
    /// bound trades against, so this counts insertions, not distinct keys.
    pub fn inserted(&self) -> u64 {
        self.inserted
    }

    /// Forget everything, the ledger included.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.inserted = 0;
        self.evicted = 0;
        self.invalidated = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::catalog::ContractSet;
    use crate::report::{KeyGroup, Unresolved};

    fn key(i: usize) -> String {
        format!("zk2/h1/tc/tc.netif.v1/stream/k{i}")
    }

    /// The bound holds and every drop is counted: nothing vanishes
    /// unaccounted.
    #[test]
    fn the_bound_holds_and_every_drop_is_counted() {
        let set = ContractSet::new();
        let lens = Lens::new("", None, &set);
        let mut cache = IdentityCache::with_capacity(100);
        for i in 0..1_000 {
            cache.ensure(&lens, &key(i));
        }
        assert!(cache.len() <= 100, "held {}", cache.len());
        assert!(cache.evicted() > 0, "the fixture must trip the bound");
        assert_eq!(cache.inserted(), 1_000, "every key here was distinct");
        assert_eq!(cache.len() as u64 + cache.evicted(), cache.inserted());
        assert_eq!(cache.invalidated(), 0);
    }

    /// The least recently ensured is the one that goes, and a key evicted
    /// and ensured again is resolved again.
    #[test]
    fn the_least_recently_ensured_goes_and_comes_back_resolved_again() {
        let set = ContractSet::new();
        let lens = Lens::new("", None, &set);
        let mut cache = IdentityCache::with_capacity(4);
        for i in 0..4 {
            cache.ensure(&lens, &key(i));
        }
        cache.ensure(&lens, &key(0));
        cache.ensure(&lens, &key(4));
        assert!(cache.get(&key(0)).is_some(), "re-ensured, so kept");
        assert!(cache.get(&key(1)).is_none(), "the oldest went");
        let before = cache.inserted();
        cache.ensure(&lens, &key(1));
        assert_eq!(cache.inserted(), before + 1, "resolved again");
        assert_eq!(
            cache.len() as u64 + cache.evicted() + cache.invalidated(),
            cache.inserted()
        );
    }

    /// A lens at another generation drops what was resolved under the old
    /// one, counted apart from the bound's evictions; `get` is pure and
    /// `clear` resets the ledger.
    #[test]
    fn another_generation_resolves_again() {
        let set = ContractSet::new();
        let lens = Lens::new("", None, &set);
        let mut cache = IdentityCache::default();
        let first = cache.ensure(&lens, &key(0)).identity.clone();
        assert_eq!(first.unresolved, Some(Unresolved::PresenceNotRead));
        cache.ensure(&lens, &key(1));
        assert_eq!(cache.generation(), 0);

        let moved = Lens::new("", None, &set).at_generation(7);
        let again = cache.ensure(&moved, "elsewhere/x").identity.clone();
        assert_eq!(again.group, KeyGroup::NotZk2);
        assert_eq!(cache.generation(), 7);
        assert_eq!(cache.invalidated(), 2, "both earlier keys dropped");
        assert_eq!(cache.evicted(), 0, "a generation is not the bound");
        assert!(cache.get(&key(0)).is_none());
        assert_eq!(cache.len() as u64 + cache.invalidated(), cache.inserted());

        let mut held: Vec<&str> = cache.keys().collect();
        held.sort_unstable();
        assert_eq!(held, ["elsewhere/x"], "keys lists what is held, no more");

        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(
            (cache.inserted(), cache.evicted(), cache.invalidated()),
            (0, 0, 0)
        );
    }
}

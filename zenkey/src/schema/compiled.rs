//! Compiled-schema caching (issue #100).
//!
//! A non-self-describing codec cannot read a byte until it has turned the
//! *served document* into an interpreter: protobuf builds a
//! `DescriptorPool` from a base64 `FileDescriptorSet`, CDR walks a JSON field
//! list into a type model. Both were doing that on **every** call, in both
//! directions — invisible while protobuf was compiled out of the binaries,
//! and squarely on the per-sample path once #97 turned the codecs on in the
//! shipped explorers.
//!
//! [`CompiledCache`] is the shared piece of the fix, keyed on
//! [`TypeSchema::hash`] — which RFC 08 §7 says "exists for client caching",
//! so this is the reuse the hash was put there for. A decoder embeds one and
//! calls [`CompiledCache::get_or_compile`]; that is the whole opt-in, and it
//! is deliberately available to an application-registered kind on the same
//! terms as the built-ins rather than being welded into the registry's
//! dispatch.
//!
//! **Drift stays observable.** The key is the hash, so a producer that
//! changes a type recompiles rather than serving a stale interpreter — the
//! same property that makes the hash a drift signal for the doctor.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use super::TypeSchema;

/// A per-decoder cache of compiled schema forms.
///
/// `T` is whatever the codec needs to interpret bytes — a message descriptor,
/// a resolved type model. Entries are handed out as `std::sync::Arc<T>` so a
/// decode borrows nothing from the cache and holds no lock while it runs.
///
/// **Bound.** One entry per distinct schema hash the process has decoded,
/// with no eviction — the same posture, and the same reasoning, as
/// `SchemaStore`'s querier map: the population is the fleet's type set, which
/// is bounded by the registry and small, and every entry here is the compiled
/// form of a document the store is already holding. If that assumption ever
/// stops holding, [`CompiledCache::len`] is what says so.
pub struct CompiledCache<T> {
    entries: Mutex<HashMap<String, std::sync::Arc<T>>>,
    /// How many times a compile actually ran. The assertion surface for
    /// #100: "built once" is a counter, not an inspection.
    compilations: AtomicU64,
}

impl<T> Default for CompiledCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> CompiledCache<T> {
    pub fn new() -> CompiledCache<T> {
        CompiledCache {
            entries: Mutex::new(HashMap::new()),
            compilations: AtomicU64::new(0),
        }
    }

    /// The compiled form for this schema, building it at most once per hash.
    ///
    /// A schema whose hash is **empty** is never cached: the hash is the
    /// identity, and caching every unhashed schema under `""` would hand one
    /// type's interpreter to another. Such a schema simply pays the compile
    /// each time, which is what it did before this cache existed.
    pub fn get_or_compile<E>(
        &self,
        schema: &TypeSchema,
        build: impl FnOnce(&TypeSchema) -> Result<T, E>,
    ) -> Result<std::sync::Arc<T>, E> {
        let key = schema.hash();
        if !key.is_empty() {
            let entries = self.entries.lock().expect("compiled cache lock");
            if let Some(hit) = entries.get(key) {
                return Ok(std::sync::Arc::clone(hit));
            }
        }
        // Compiled outside the lock: a slow build must not block every other
        // type's decodes. Two threads racing the same miss both build and the
        // last insert wins — one wasted compile, never a wrong answer.
        self.compilations.fetch_add(1, Ordering::Relaxed);
        let compiled = std::sync::Arc::new(build(schema)?);
        if !key.is_empty() {
            let mut entries = self.entries.lock().expect("compiled cache lock");
            entries.insert(key.to_string(), std::sync::Arc::clone(&compiled));
        }
        Ok(compiled)
    }

    /// How many compiles have run — one per distinct schema, if the cache is
    /// doing its job.
    pub fn compilations(&self) -> u64 {
        self.compilations.load(Ordering::Relaxed)
    }

    /// Distinct compiled forms retained.
    pub fn len(&self) -> usize {
        self.entries.lock().expect("compiled cache lock").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drop every compiled form, keeping the counters.
    ///
    /// Not needed in normal operation — the hash key already handles drift —
    /// but a long-running tool that has walked a very wide bus can reclaim.
    pub fn clear(&self) {
        self.entries.lock().expect("compiled cache lock").clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A schema with a chosen hash, built through the served-document parser
    /// so the fixture is the shape a producer actually serves.
    fn schema(hash: &str) -> TypeSchema {
        let doc = serde_json::json!({
            "schema_version": 1,
            "app": "t",
            "types": {"T": {"kind": "json-schema", "hash": hash, "schema": {}}},
        });
        super::super::SchemaSet::parse(&doc.to_string())
            .expect("fixture set")
            .get("T")
            .expect("fixture type")
            .clone()
    }

    #[test]
    fn one_compile_per_hash_however_many_calls() {
        let cache: CompiledCache<u32> = CompiledCache::new();
        let s = schema("sha256:aaa");
        for _ in 0..100 {
            let got = cache
                .get_or_compile(&s, |_| Ok::<_, ()>(7))
                .expect("compiles");
            assert_eq!(*got, 7);
        }
        assert_eq!(
            cache.compilations(),
            1,
            "the interpreter must be built once, not per call"
        );
        assert_eq!(cache.len(), 1);
    }

    /// Drift must stay observable: a changed hash is a different type, and
    /// serving the old interpreter for it would hide exactly what the hash
    /// exists to reveal.
    #[test]
    fn a_changed_hash_recompiles() {
        let cache: CompiledCache<u32> = CompiledCache::new();
        let old = cache
            .get_or_compile(&schema("sha256:aaa"), |_| Ok::<_, ()>(1))
            .unwrap();
        let new = cache
            .get_or_compile(&schema("sha256:bbb"), |_| Ok::<_, ()>(2))
            .unwrap();
        assert_eq!((*old, *new), (1, 2));
        assert_eq!(cache.compilations(), 2);
        assert_eq!(cache.len(), 2);
    }

    /// An unhashed schema has no identity to cache under; caching it as `""`
    /// would hand one type's interpreter to the next.
    #[test]
    fn an_unhashed_schema_is_never_cached() {
        let cache: CompiledCache<u32> = CompiledCache::new();
        let mut seq = 0;
        for _ in 0..3 {
            seq += 1;
            let got = cache
                .get_or_compile(&schema(""), |_| Ok::<_, ()>(seq))
                .unwrap();
            assert_eq!(
                *got, seq,
                "each call gets its own build, never a neighbour's"
            );
        }
        assert_eq!(cache.compilations(), 3);
        assert!(cache.is_empty(), "nothing is retained under an empty key");
    }

    /// A failed compile is not cached — the next call retries rather than
    /// inheriting a failure that may have been transient in the document.
    #[test]
    fn a_failed_compile_leaves_nothing_behind() {
        let cache: CompiledCache<u32> = CompiledCache::new();
        let s = schema("sha256:aaa");
        assert!(cache.get_or_compile(&s, |_| Err::<u32, _>("bad")).is_err());
        assert!(cache.is_empty());
        assert_eq!(*cache.get_or_compile(&s, |_| Ok::<_, ()>(9)).unwrap(), 9);
        assert_eq!(cache.compilations(), 2);
    }

    #[test]
    fn clearing_keeps_the_compile_count() {
        let cache: CompiledCache<u32> = CompiledCache::new();
        cache
            .get_or_compile(&schema("sha256:aaa"), |_| Ok::<_, ()>(1))
            .unwrap();
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.compilations(), 1);
    }
}

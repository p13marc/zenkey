# Draft upstream report: the storage manager accepts outdated samples after a delete

**Status: a draft, not filed.** Filing it in eclipse-zenoh/zenoh waits for the
maintainer's go-ahead (#601, #605). The measurements are
[`../spike-report.md`](../spike-report.md) § S5, and the raw data is
[`../spike-results/s5/`](../spike-results/s5/) (stock) and
[`../spike-results/s5-patched/`](../spike-results/s5-patched/) (with the patch
below).

---

**Title:** storage manager: an older put after a delete resurrects the key
(`guard_cache_if_latest` falls through when the cache holds a newer event)

**Version:** zenoh / zenoh-plugin-storage-manager 1.10.1, memory volume.

**What happens.** A storage that has processed a delete of `k` (timestamp T2)
accepts a later-arriving put on `k` whose timestamp T0 is older than T2. A GET
then returns the resurrected value. It happens:
- without replication, at once (0.3 s after the delete) and after a GC tick;
- with replication, within about one replication interval of the delete.

The same path makes a wildcard delete followed by an older put on one of its
keys leave an empty-payload value (the class of #2649).

**Minimal reproduction** (router with
`storage_manager: { storages: { demo: { key_expr: "demo/**", volume: "memory" } } }`,
timestamping on):
1. A client puts `demo/a` = `v1` with timestamp T1 (now).
2. It deletes `demo/a` with timestamp T2 (now, > T1).
3. Another client puts `demo/a` = `v0` with an explicit timestamp T0 = T1 − 1 ms.
4. `get("demo/a")` returns `v0`. Expected: nothing, since `v0` is older than the delete.

**Cause** (`src/storages_mgt/service.rs`, `guard_cache_if_latest`):

```rust
if let Some(event) = cache_guard.get(&new_event.log_key()) {
    if new_event.timestamp > event.timestamp {
        return Some(cache_guard);
    }
    // missing: the cache holds a newer event for this key → outdated
}
// falls through to the replication log (which may not have the delete
// yet) or to the stored value (gone after a delete) → accepted
```

**A second, independent problem.** The garbage collection retains the
wrong side of the limit:
`latest_updates.retain(|_, e| e.timestamp().get_time() < &time_limit)` keeps
events *older* than `now − lifespan` and drops the recent ones. Without
replication, the cache therefore loses every recent tombstone at each GC tick.

**Patch** ([`storage-manager-outdated-guard.patch`](storage-manager-outdated-guard.patch)):

```diff
             if new_event.timestamp > event.timestamp {
                 return Some(cache_guard);
             }
+            return None;
         }
 ...
-                .retain(|_, event| event.timestamp().get_time() < &time_limit);
+                .retain(|_, event| event.timestamp().get_time() >= &time_limit);
```

**Measured effect** (the zk2 spike's S5, 42 cases):
- **Stock:** 10 wrong answers.
- **Patched:** 6. The 6 that remain are design-level and not storage bugs: a stale storage answering alone, a producer restarting with its clock behind, a tombstone window shorter than the storage's staleness, an unstamped producer, and the memory backend ignoring `_time`.
- **Every storage-caused case is fixed:** the late older put (with and without replication, before and after GC), the wildcard delete followed by a stale put, and an older put replacing a newer value with replication.

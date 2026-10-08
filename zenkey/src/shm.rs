//! Shared memory's one trap (spec §7.4): zenoh-shm 1.10.1 locks every
//! segment it creates or maps, and below the pool's need SHM silently falls
//! back to TCP (spike S9: under an 8 MiB `RLIMIT_MEMLOCK`). The runtime
//! cannot see the fallback, so it checks the limit and says so.

/// The soft `RLIMIT_MEMLOCK`, in bytes; `None` when unlimited or unknown.
#[must_use]
pub fn memlock_limit() -> Option<u64> {
    let mut r = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` writes one `rlimit` through a valid pointer.
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, &raw mut r) };
    (rc == 0 && r.rlim_cur != libc::RLIM_INFINITY).then_some(r.rlim_cur)
}

/// The limit spike S9 saw SHM fall back under. A pool needs at least its own
/// size plus 2 MiB (§7.4); this is the floor below which no useful pool fits.
pub const MEMLOCK_FLOOR: u64 = 8 * 1024 * 1024;

/// Warns once per process when the memlock limit is below
/// [`MEMLOCK_FLOOR`]: an `@stream` writer's SHM buffers would fall back to
/// TCP without a word from zenoh.
pub(crate) fn warn_if_memlock_low() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Some(limit) = memlock_limit().filter(|l| *l < MEMLOCK_FLOOR) {
            tracing::warn!(
                limit,
                floor = MEMLOCK_FLOOR,
                "RLIMIT_MEMLOCK is below what a shared-memory pool needs: SHM falls back to \
                 TCP silently (spec §7.4); raise it to the pool size plus 2 MiB"
            );
        }
    });
}

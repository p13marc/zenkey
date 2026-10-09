//! Shared memory's one trap (spec §7.4): zenoh-shm 1.10.1 locks every
//! segment it creates or maps, and below the pool's need SHM silently falls
//! back to TCP (spike S9: under an 8 MiB `RLIMIT_MEMLOCK`). The runtime
//! cannot see the fallback, so it checks the limit and says so.

/// The soft `RLIMIT_MEMLOCK`, as this process reads it (#677).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Memlock {
    /// A limit, in bytes.
    Limited(u64),
    /// No limit.
    Unlimited,
    /// The limit could not be read: neither a limit nor its absence is
    /// known.
    Unknown,
}

/// The soft `RLIMIT_MEMLOCK`: a limit, none, or unknown (§7.4).
#[must_use]
pub fn memlock() -> Memlock {
    let mut r = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` writes one `rlimit` through a valid pointer.
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, &raw mut r) };
    match (rc, r.rlim_cur) {
        (0, libc::RLIM_INFINITY) => Memlock::Unlimited,
        (0, limit) => Memlock::Limited(limit),
        _ => Memlock::Unknown,
    }
}

/// The soft `RLIMIT_MEMLOCK`, in bytes; `None` when unlimited **or
/// unknown**, which [`memlock`] tells apart.
#[must_use]
pub fn memlock_limit() -> Option<u64> {
    match memlock() {
        Memlock::Limited(l) => Some(l),
        Memlock::Unlimited | Memlock::Unknown => None,
    }
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
        if let Memlock::Limited(limit) = memlock()
            && limit < MEMLOCK_FLOOR
        {
            tracing::warn!(
                limit,
                floor = MEMLOCK_FLOOR,
                "RLIMIT_MEMLOCK is below what a shared-memory pool needs: SHM falls back to \
                 TCP silently (spec §7.4); raise it to the pool size plus 2 MiB"
            );
        }
    });
}

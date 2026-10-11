//! What a window's reader sees of its own receive path (core §2.7, 0.27;
//! #753): "its reader lost none" also needs its receive path never to have
//! stopped long enough for a router to give up on it, over the window and
//! one lease after it.
//!
//! A router's push of a `reliable`, `block` sample waits for room in the
//! reader's queue for `wait_before_close` (5 s by default), then closes the
//! transport, and what it held for the reader goes with it. The reader is
//! told nothing it can count: it sees the owner's instance token withdrawn
//! only if it had received that token before it stalled, which in 4 of 27
//! measured runs it had not (Appendix B). Two signals stand in, both
//! read by [`StallWatch`]:
//!
//! - **the session's own transports closing**, as zenoh's transport events
//!   say (`Session::info().transport_events_listener()`, unstable in
//!   1.10.1). A router giving up on this reader closes the reader's own
//!   transport, and the session sees it once its receive threads run
//!   again: in every measured run, a stuck callback's and a stopped
//!   process's alike;
//! - **a monotonic ticker on a thread of its own**, whose greatest gap
//!   bounds how long the process did not run (stopped, starved, swapped
//!   out). It sees such a stall the moment the process runs again, before
//!   the session has read its transport's close; and it does not see
//!   zenoh's receive threads blocked while the rest of the process runs,
//!   which only the first signal does.
//!
//! The verdict is the judge's, by `zenkey_model::budget::lossless`, from
//! the [`StallSeen`] this returns and the deployment's bound
//! (`zenkey_model::budget::stall_bound`).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zenoh::Session;

/// How often the ticker ticks. The gap it reports is a stall plus at most
/// one period, so a period well under any bound keeps the bound honest.
pub const TICK: Duration = Duration::from_millis(50);

/// What a [`StallWatch`] saw, from its start to its read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StallSeen {
    /// The greatest gap between two ticks, the one still open at the read
    /// included: how long this process did not run, at most.
    pub gap: Duration,
    /// How many transports of the session closed.
    pub transports_closed: u64,
    /// How long it watched.
    pub watched: Duration,
}

/// The ticker's state, shared with its thread.
#[derive(Debug)]
struct Ticks {
    last: Instant,
    worst: Duration,
}

/// The ticker and the transport-event listener of one window's reader:
/// started before the window opens, read once its watch of the owner ends.
/// Dropping it stops the ticker and undeclares the listener.
pub struct StallWatch {
    ticks: Arc<Mutex<Ticks>>,
    stop: Arc<AtomicBool>,
    closed: Arc<AtomicU64>,
    started: Instant,
    _events: zenoh::session::TransportEventsListener<()>,
}

impl std::fmt::Debug for StallWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StallWatch")
            .field("ticks", &self.ticks)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl StallWatch {
    /// The listener on `session`'s transport events, then the ticker.
    pub async fn start(session: &Session) -> std::result::Result<StallWatch, String> {
        let closed = Arc::new(AtomicU64::new(0));
        let c = Arc::clone(&closed);
        let events = session
            .info()
            .transport_events_listener()
            .callback(move |e| {
                if e.kind() == zenoh::sample::SampleKind::Delete {
                    c.fetch_add(1, Ordering::Relaxed);
                }
            })
            .await
            .map_err(|e| format!("a listener on this session's transport events: {e}"))?;
        let now = Instant::now();
        let ticks = Arc::new(Mutex::new(Ticks {
            last: now,
            worst: Duration::ZERO,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (t, s) = (Arc::clone(&ticks), Arc::clone(&stop));
        std::thread::Builder::new()
            .name("zk2-stall-ticker".to_owned())
            .spawn(move || {
                while !s.load(Ordering::Relaxed) {
                    std::thread::sleep(TICK);
                    let now = Instant::now();
                    let mut t = t.lock().expect("not poisoned");
                    t.worst = t.worst.max(now.saturating_duration_since(t.last));
                    t.last = now;
                }
            })
            .map_err(|e| format!("the stall ticker's thread: {e}"))?;
        Ok(StallWatch {
            ticks,
            stop,
            closed,
            started: now,
            _events: events,
        })
    }

    /// What it saw so far.
    pub fn read(&self) -> StallSeen {
        let t = self.ticks.lock().expect("not poisoned");
        StallSeen {
            gap: t.worst.max(t.last.elapsed()),
            transports_closed: self.closed.load(Ordering::Relaxed),
            watched: self.started.elapsed(),
        }
    }
}

impl Drop for StallWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

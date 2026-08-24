//! The retained window (issue #217): a bounded ring of recent samples on
//! the monitor's ingest path, so "scrub back to before you noticed" needs
//! no recording to have been running.
//!
//! Two budgets, both in force at once — **bytes** and **duration** — because
//! each fails alone: a byte budget on a quiet bus retains stale hours, a
//! duration budget on a hot one retains an unbounded burst. The defaults are
//! deliberately small ([`RetentionBudget::default`]: 64 MiB / 2 min) — a
//! generous default here turns an overnight session into a memory incident.
//!
//! **What eviction from this ring is, and is not** (RFC 09 §5.1 **O6**,
//! sharpened at ratification — v1.18 R1): a bounded observer reports what
//! each bound cost and MUST NOT fold the kinds into one number. The ring's
//! costs are therefore counted apart from every existing population — from
//! broadcast lag ([`crate::MonitorCore::dropped`], "could not keep up"),
//! from stats-table eviction ([`crate::model::stats::StatsTable::evicted`], "chose
//! to forget under the key bound") and from unwatch retirement
//! ([`crate::model::stats::StatsTable::unwatched`], "stopped looking, by request")
//! — and the ring itself keeps its own two kinds apart:
//!
//! - [`RetentionStats::evicted`] — samples dropped because the **byte**
//!   budget bit. The window is then *narrower than the age claim*, which is
//!   exactly what a consumer must be told before trusting "the last 2 min".
//! - [`RetentionStats::expired`] — samples that aged past the duration
//!   budget: the window sliding exactly as declared, not a loss against the
//!   claim, and still counted rather than silently absorbed.
//!
//! The ring holds `Arc<SampleView>` — retaining a sample is a refcount bump
//! on zenoh's refcounted buffers, not a copy (`docs/zero-copy.md` §4); the
//! byte budget accounts the payload bytes those Arcs keep alive.
//!
//! ## Why the ring is chunked (#331)
//!
//! The ring lives behind [`crate::MonitorCore`]'s retain mutex, which
//! `ingest` takes on **zenoh's network callback thread**. A read that cloned
//! the whole `VecDeque` therefore stalled the network layer for one refcount
//! atomic per retained sample — ~260 000 of them at the default budget — and
//! zengui called it from `update()`, twice in a row, per retained-window
//! entry.
//!
//! So the ring is a queue of **sealed, immutable chunks** (1024 samples
//! each) plus one open tail. A read clones the chunk pointers and the tail
//! (`RetainedParts`) — bounded by `window / CHUNK + CHUNK` pointer clones,
//! ~1 300 atomics at the same budget, and *nothing* that grows with the
//! payload — and flattens them into the handed-out `Arc<[_]>` after the lock
//! is released. Pushes stay O(1) amortised: a chunk is sealed once per
//! `CHUNK` samples, which is a `drain` into an `Arc<[_]>` and nothing else.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::bus::monitor::SampleView;

/// The two bounds on the retained window, both always in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionBudget {
    /// Accounted bytes the ring may hold ([`sample_cost`] per sample).
    pub max_bytes: usize,
    /// How far back the window reaches, on the observer's arrival clock
    /// ([`SampleView::received`]).
    pub max_age: Duration,
}

impl Default for RetentionBudget {
    /// 64 MiB / 2 min — small on purpose (#217): the budget is visible in
    /// the GUI's status strip, and an operator who wants more says so.
    fn default() -> RetentionBudget {
        RetentionBudget {
            max_bytes: 64 * 1024 * 1024,
            max_age: Duration::from_secs(120),
        }
    }
}

/// What the ring holds and what its bounds have cost, as of one read.
///
/// All `Copy`: the monitor hands this out per stats tick, and a snapshot
/// that allocated would contend with the ingest path for nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionStats {
    /// The budget in force — the banner states it (#217).
    pub budget: RetentionBudget,
    /// Samples currently retained.
    pub retained: usize,
    /// Accounted bytes currently retained.
    pub retained_bytes: usize,
    /// Oldest retained sample → newest, on the arrival clock. Zero when
    /// fewer than two samples are held. When [`RetentionStats::evicted`] is
    /// non-zero this is shorter than `budget.max_age` claims — which is why
    /// both ride together.
    pub span: Duration,
    /// Samples dropped because the **byte** budget bit (O6: the bound's
    /// cost). Counted apart from [`RetentionStats::expired`], from broadcast
    /// lag, and from both stats-table populations — v1.18 R1 forbids the
    /// fold.
    pub evicted: u64,
    /// Samples that aged past `budget.max_age` — the window sliding as
    /// declared.
    pub expired: u64,
}

/// What one retained sample costs the byte budget: the bytes its `Arc`
/// keeps alive (payload, attachment, key) plus a flat allowance for the
/// view struct itself. An estimate on the honest side of exact — a budget
/// that ignored what it stores would stop being a bound (O6).
pub fn sample_cost(view: &SampleView) -> usize {
    const OVERHEAD: usize = 160;
    view.key.len()
        + view.payload.len()
        + view.encoding.len()
        + view
            .attachment
            .as_ref()
            .map_or(0, zenoh::bytes::ZBytes::len)
        + OVERHEAD
}

/// Samples per sealed chunk — the granularity a read pays for (#331).
///
/// 1024 is the trade: a read clones `window / 1024` chunk pointers plus at
/// most 1024 tail pointers, and a push seals a chunk once per 1024 samples.
/// Both sides of that stay in the low thousands of atomics at any budget an
/// explorer is given.
pub(crate) const CHUNK: usize = 1024;

/// A read of the ring, taken **under** the mutex and flattened outside it
/// (#331): sealed chunk pointers, how far into the first one the window
/// starts, and a copy of the open tail. Cloning this is bounded by
/// `window / CHUNK + CHUNK` pointer clones; nothing in it walks the window.
pub(crate) struct RetainedParts {
    sealed: Vec<Arc<[Arc<SampleView>]>>,
    front: usize,
    tail: Vec<Arc<SampleView>>,
    len: usize,
}

impl RetainedParts {
    /// How many sealed chunk pointers this read holds, plus one for the
    /// tail — the whole cost paid under the ingest mutex, and the number a
    /// test can assert instead of a stopwatch (#331). Test-only: the
    /// production path never needs to count what it is about to flatten.
    #[cfg(test)]
    pub(crate) fn chunks(&self) -> usize {
        self.sealed.len() + 1
    }

    /// The window, oldest first. O(window) — which is why it happens with
    /// the ingest mutex released.
    pub(crate) fn flatten(self) -> Arc<[Arc<SampleView>]> {
        let mut out: Vec<Arc<SampleView>> = Vec::with_capacity(self.len);
        for (i, chunk) in self.sealed.iter().enumerate() {
            let from = if i == 0 { self.front } else { 0 };
            out.extend(chunk[from..].iter().cloned());
        }
        out.extend(self.tail);
        Arc::from(out)
    }
}

/// The ring itself. Owned by [`crate::MonitorCore`] behind its own mutex;
/// everything here is synchronous and allocation-light.
///
/// Sealed chunks and an open tail rather than one `VecDeque`, so that a read
/// is bounded work under that mutex — see the module header (#331).
#[derive(Debug)]
pub(crate) struct Retention {
    budget: RetentionBudget,
    /// Sealed chunks, oldest first. Immutable once sealed, which is what
    /// makes handing one out a pointer clone.
    sealed: VecDeque<Arc<[Arc<SampleView>]>>,
    /// Samples already evicted from the front of the oldest sealed chunk.
    front: usize,
    /// The chunk being filled. Sealed at [`CHUNK`] samples.
    tail: VecDeque<Arc<SampleView>>,
    /// Samples held across both — `sealed` cannot report its own length
    /// cheaply once `front` is non-zero.
    len: usize,
    bytes: usize,
    evicted: u64,
    expired: u64,
}

impl Retention {
    pub(crate) fn new(budget: RetentionBudget) -> Retention {
        Retention {
            budget,
            sealed: VecDeque::new(),
            front: 0,
            tail: VecDeque::new(),
            len: 0,
            bytes: 0,
            evicted: 0,
            expired: 0,
        }
    }

    /// Change the budget in force; the next push or read applies it.
    pub(crate) fn set_budget(&mut self, budget: RetentionBudget) {
        self.budget = budget;
    }

    /// Retain one sample, then enforce both budgets (oldest out first).
    pub(crate) fn push(&mut self, view: Arc<SampleView>, now: Instant) {
        self.bytes += sample_cost(&view);
        self.tail.push_back(view);
        self.len += 1;
        if self.tail.len() >= CHUNK {
            self.sealed.push_back(self.tail.drain(..).collect());
        }
        self.expire(now);
        while self.bytes > self.budget.max_bytes && self.len > 1 {
            self.pop_front();
            self.evicted += 1;
        }
        // A single sample larger than the whole budget is retained anyway
        // and honestly accounted: a window that silently held nothing would
        // read as a quiet bus.
    }

    /// The oldest retained sample, wherever it lives.
    fn oldest(&self) -> Option<&Arc<SampleView>> {
        match self.sealed.front() {
            Some(chunk) => chunk.get(self.front),
            None => self.tail.front(),
        }
    }

    /// The newest retained sample, wherever it lives.
    fn newest(&self) -> Option<&Arc<SampleView>> {
        match self.tail.back() {
            Some(view) => Some(view),
            None => self.sealed.back().and_then(|chunk| chunk.last()),
        }
    }

    fn pop_front(&mut self) {
        let popped = match self.sealed.front() {
            Some(chunk) => {
                let view = Arc::clone(&chunk[self.front]);
                self.front += 1;
                if self.front >= chunk.len() {
                    self.sealed.pop_front();
                    self.front = 0;
                }
                Some(view)
            }
            None => self.tail.pop_front(),
        };
        if let Some(v) = popped {
            self.bytes = self.bytes.saturating_sub(sample_cost(&v));
            self.len -= 1;
        }
    }

    /// Age out everything past the duration budget.
    fn expire(&mut self, now: Instant) {
        while self
            .oldest()
            .is_some_and(|v| now.saturating_duration_since(v.received) > self.budget.max_age)
        {
            self.pop_front();
            self.expired += 1;
        }
    }

    /// The window as chunk pointers — bounded work, for the caller to
    /// flatten once the mutex is released (#331).
    pub(crate) fn parts(&mut self, now: Instant) -> RetainedParts {
        self.expire(now);
        RetainedParts {
            sealed: self.sealed.iter().map(Arc::clone).collect(),
            front: self.front,
            tail: self.tail.iter().map(Arc::clone).collect(),
            len: self.len,
        }
    }

    /// The window, oldest first — both halves in one call, for the module's
    /// own tests and for callers that hold the ring exclusively.
    #[cfg(test)]
    pub(crate) fn snapshot(&mut self, now: Instant) -> Arc<[Arc<SampleView>]> {
        self.parts(now).flatten()
    }

    /// The window's account of itself, budgets applied as of `now`.
    pub(crate) fn stats(&mut self, now: Instant) -> RetentionStats {
        self.expire(now);
        let span = match (self.oldest(), self.newest()) {
            (Some(oldest), Some(newest)) => {
                newest.received.saturating_duration_since(oldest.received)
            }
            _ => Duration::ZERO,
        };
        RetentionStats {
            budget: self.budget,
            retained: self.len,
            retained_bytes: self.bytes,
            span,
            evicted: self.evicted,
            expired: self.expired,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenoh::sample::SampleKind;

    fn view(key: &str, len: usize, received: Instant) -> Arc<SampleView> {
        Arc::new(SampleView {
            key: key.to_string(),
            payload: zenoh::bytes::ZBytes::from(vec![0u8; len]),
            encoding: String::new(),
            kind: SampleKind::Put,
            timestamp: None,
            stamped_by: None,
            attachment: None,
            priority: zenoh::qos::Priority::DEFAULT,
            congestion_control: zenoh::qos::CongestionControl::DEFAULT,
            reliability: zenoh::qos::Reliability::DEFAULT,
            express: false,
            source: None,
            received,
        })
    }

    /// The default is the small one the issue names — and the banner states
    /// it, so a silent change here would make the banner lie.
    #[test]
    fn the_default_budget_is_64_mib_and_two_minutes() {
        let b = RetentionBudget::default();
        assert_eq!(b.max_bytes, 64 * 1024 * 1024);
        assert_eq!(b.max_age, Duration::from_secs(120));
    }

    /// The byte budget evicts oldest-first and counts what it cost (O6).
    #[test]
    fn the_byte_budget_evicts_oldest_first_and_counts() {
        let now = Instant::now();
        let mut r = Retention::new(RetentionBudget {
            max_bytes: 3 * sample_cost(&view("k", 100, now)),
            max_age: Duration::from_secs(3600),
        });
        for i in 0..10 {
            r.push(view(&format!("k{i}"), 100 - i, now), now);
        }
        let s = r.stats(now);
        assert!(s.retained < 10, "the bound bit");
        assert_eq!(s.retained as u64 + s.evicted, 10, "present or counted");
        assert_eq!(s.expired, 0, "nothing aged out — the kinds stay apart");
        let kept = r.snapshot(now);
        assert_eq!(kept.first().unwrap().key, format!("k{}", 10 - kept.len()));
        assert_eq!(kept.last().unwrap().key, "k9", "newest survives");
    }

    /// The duration budget is the window sliding as declared — counted as
    /// `expired`, never folded into `evicted` (v1.18 R1).
    #[test]
    fn aging_out_is_expiry_not_eviction() {
        let t0 = Instant::now();
        let mut r = Retention::new(RetentionBudget {
            max_bytes: usize::MAX,
            max_age: Duration::from_secs(10),
        });
        r.push(view("old", 4, t0), t0);
        r.push(
            view("new", 4, t0 + Duration::from_secs(20)),
            t0 + Duration::from_secs(20),
        );
        let s = r.stats(t0 + Duration::from_secs(20));
        assert_eq!(s.retained, 1);
        assert_eq!(s.expired, 1);
        assert_eq!(s.evicted, 0, "no byte bound bit — two kinds, two numbers");
        assert_eq!(r.snapshot(t0 + Duration::from_secs(20))[0].key, "new");
    }

    /// A sample larger than the whole byte budget is retained and accounted
    /// rather than silently refused — an empty window must never be
    /// manufactured by the bound.
    #[test]
    fn one_oversized_sample_is_held_not_hidden() {
        let now = Instant::now();
        let mut r = Retention::new(RetentionBudget {
            max_bytes: 8,
            max_age: Duration::from_secs(3600),
        });
        r.push(view("big", 1024, now), now);
        let s = r.stats(now);
        assert_eq!(s.retained, 1);
        assert!(
            s.retained_bytes > s.budget.max_bytes,
            "over budget, and said so"
        );
    }

    /// The chunking is invisible from the outside (#331): order, length and
    /// the span read the same across a sealed boundary as inside one chunk.
    #[test]
    fn the_window_reads_the_same_across_chunk_boundaries() {
        let now = Instant::now();
        let mut r = Retention::new(RetentionBudget {
            max_bytes: usize::MAX,
            max_age: Duration::from_secs(3600),
        });
        let total = CHUNK * 2 + 7;
        for i in 0..total {
            r.push(view(&format!("k{i:05}"), 8, now), now);
        }
        let kept = r.snapshot(now);
        assert_eq!(kept.len(), total);
        assert_eq!(kept[0].key, "k00000");
        assert_eq!(kept[CHUNK].key, format!("k{CHUNK:05}"), "the seam holds");
        assert_eq!(kept.last().unwrap().key, format!("k{:05}", total - 1));
        assert_eq!(r.stats(now).retained, total);
    }

    /// Eviction walks *into* a sealed chunk rather than dropping it whole:
    /// the byte budget's granularity is one sample, chunked or not.
    #[test]
    fn eviction_walks_into_a_sealed_chunk() {
        let now = Instant::now();
        let keep = CHUNK + 5;
        let mut r = Retention::new(RetentionBudget {
            max_bytes: sample_cost(&view("k00000", 8, now)) * keep,
            max_age: Duration::from_secs(3600),
        });
        let total = CHUNK * 3;
        for i in 0..total {
            r.push(view(&format!("k{i:05}"), 8, now), now);
        }
        let s = r.stats(now);
        assert_eq!(s.retained, keep, "the bound bit mid-chunk");
        assert_eq!(
            s.retained as u64 + s.evicted,
            total as u64,
            "present or counted"
        );
        let kept = r.snapshot(now);
        assert_eq!(kept.len(), keep);
        assert_eq!(kept[0].key, format!("k{:05}", total - keep));
        assert_eq!(kept.last().unwrap().key, format!("k{:05}", total - 1));
    }

    /// The span states what the window actually holds — which is shorter
    /// than the age claim exactly when `evicted` is non-zero.
    #[test]
    fn the_span_is_measured_not_claimed() {
        let t0 = Instant::now();
        let mut r = Retention::new(RetentionBudget::default());
        r.push(view("a", 4, t0), t0);
        r.push(
            view("b", 4, t0 + Duration::from_secs(30)),
            t0 + Duration::from_secs(30),
        );
        assert_eq!(
            r.stats(t0 + Duration::from_secs(30)).span,
            Duration::from_secs(30)
        );
    }
}

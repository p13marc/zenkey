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

/// The ring itself. Owned by [`crate::MonitorCore`] behind its own mutex;
/// everything here is synchronous and allocation-light.
#[derive(Debug)]
pub(crate) struct Retention {
    budget: RetentionBudget,
    ring: VecDeque<Arc<SampleView>>,
    bytes: usize,
    evicted: u64,
    expired: u64,
}

impl Retention {
    pub(crate) fn new(budget: RetentionBudget) -> Retention {
        Retention {
            budget,
            ring: VecDeque::new(),
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
        self.ring.push_back(view);
        self.expire(now);
        while self.bytes > self.budget.max_bytes && self.ring.len() > 1 {
            self.pop_front();
            self.evicted += 1;
        }
        // A single sample larger than the whole budget is retained anyway
        // and honestly accounted: a window that silently held nothing would
        // read as a quiet bus.
    }

    fn pop_front(&mut self) {
        if let Some(v) = self.ring.pop_front() {
            self.bytes = self.bytes.saturating_sub(sample_cost(&v));
        }
    }

    /// Age out everything past the duration budget.
    fn expire(&mut self, now: Instant) {
        while self
            .ring
            .front()
            .is_some_and(|v| now.saturating_duration_since(v.received) > self.budget.max_age)
        {
            self.pop_front();
            self.expired += 1;
        }
    }

    /// The window, oldest first — `Arc` clones, not copies.
    pub(crate) fn snapshot(&mut self, now: Instant) -> Vec<Arc<SampleView>> {
        self.expire(now);
        self.ring.iter().cloned().collect()
    }

    /// The window's account of itself, budgets applied as of `now`.
    pub(crate) fn stats(&mut self, now: Instant) -> RetentionStats {
        self.expire(now);
        let span = match (self.ring.front(), self.ring.back()) {
            (Some(oldest), Some(newest)) => {
                newest.received.saturating_duration_since(oldest.received)
            }
            _ => Duration::ZERO,
        };
        RetentionStats {
            budget: self.budget,
            retained: self.ring.len(),
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

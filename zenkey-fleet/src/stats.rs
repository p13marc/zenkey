//! Windowed per-key statistics (issues #13/#15): message/byte counters and
//! an exponentially-weighted rate, keyed by wire key. Backs `zenctl topic
//! hz`/`bw`/`echo --rate` and zengui's tree badges.
//!
//! Perf posture (report §14): lookups borrow (`&str` against the `String`
//! keys — no per-sample allocation on the hot hit path); one allocation per
//! *new* key is the floor.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// How many per-key latency observations the summary window keeps. Bounded
/// like everything else an hours-long observer accumulates (O6).
const LAT_WINDOW: usize = 256;

/// The observed **skewed** latency distribution of one key (#119):
/// (arrival wall-clock − publisher HLC), µs, over the last [`LAT_WINDOW`]
/// stamped samples.
///
/// The caveat is part of the measurement: this contains clock skew, and
/// HLCs are only as good as the fleet's time discipline. Negative values
/// are the skew *evidence* and are never clamped — render this as
/// "observed skewed latency", an observation, not a verdict on the
/// transport (RFC 09 §5.1 applied to a number).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct LatencySummary {
    pub min_us: i64,
    pub median_us: i64,
    pub p95_us: i64,
    pub max_us: i64,
    /// Stamped samples in the window.
    pub samples: usize,
}

/// One key's running statistics.
#[derive(Debug, Clone)]
pub struct KeyStats {
    pub count: u64,
    pub bytes: u64,
    /// EWMA of the instantaneous rate (Hz), time-decayed.
    pub rate_hz: f64,
    pub last_seen: Instant,
    /// Consecutive source-sequence-number gap count, when publishers attach
    /// SourceInfo (unstable API) — loss visibility, `--loss`.
    pub sn_gaps: u64,
    /// Samples that carried **no** HLC timestamp — an observation of their
    /// own, counted separately: an unstamped sample has no latency, which
    /// is not the same as zero latency (#119).
    pub unstamped: u64,
    last_sn: Option<u32>,
    /// Bounded window of observed skewed latencies, µs.
    lat: VecDeque<i64>,
}

impl KeyStats {
    /// The window's distribution, or `None` before any stamped sample.
    pub fn latency(&self) -> Option<LatencySummary> {
        if self.lat.is_empty() {
            return None;
        }
        let mut sorted: Vec<i64> = self.lat.iter().copied().collect();
        sorted.sort_unstable();
        let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q) as usize];
        Some(LatencySummary {
            min_us: sorted[0],
            median_us: at(0.5),
            p95_us: at(0.95),
            max_us: *sorted.last().expect("non-empty"),
            samples: sorted.len(),
        })
    }
}

/// The table. Feed it samples; read it per key or in aggregate.
///
/// **Bounded.** A CLI runs for `--window` seconds and exits, so an unbounded
/// map was fine; a GUI left open overnight on a bus carrying content-addressed
/// or per-request keys would grow one entry per key forever. The table
/// therefore keeps at most [`DEFAULT_MAX_KEYS`] entries, evicting the
/// least-recently-seen first — the keys that stopped publishing are the ones a
/// live view has least use for — and **counts every eviction**, so a shrinking
/// key set is never mistaken for a quiet bus (RFC 09 §5.1).
#[derive(Debug)]
pub struct StatsTable {
    keys: HashMap<String, KeyStats>,
    max_keys: usize,
    evicted: u64,
    unwatched: u64,
}

impl Default for StatsTable {
    fn default() -> Self {
        StatsTable::with_capacity(DEFAULT_MAX_KEYS)
    }
}

/// EWMA time constant (~2 s: responsive enough for a UI badge, smooth
/// enough not to flicker); samples older than ~tau contribute e^-1.
const TAU: Duration = Duration::from_secs(2);

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

impl StatsTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// A table bounded at `max_keys` entries.
    pub fn with_capacity(max_keys: usize) -> Self {
        StatsTable {
            keys: HashMap::new(),
            max_keys: max_keys.max(1),
            evicted: 0,
            unwatched: 0,
        }
    }

    /// Keys dropped to stay within the bound.
    ///
    /// Non-zero means the view is partial: some keys that carried traffic are
    /// no longer represented in [`len`](Self::len), [`totals`](Self::totals) or
    /// any tree built from this table.
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    /// The bound in force.
    pub fn max_keys(&self) -> usize {
        self.max_keys
    }

    /// Keys retired because no active watch covers them any more
    /// ([`retire_unwatched`](Self::retire_unwatched)).
    ///
    /// The third O6 category, deliberately distinct from
    /// [`evicted`](Self::evicted) ("chose to forget under the bound") and the
    /// broadcast's dropped ("could not keep up"): this one is "stopped
    /// looking, by request" — and a key set that shrinks because the user
    /// unwatched a subtree must say so, or it reads as a quieting bus.
    pub fn unwatched(&self) -> u64 {
        self.unwatched
    }

    /// Retire every key that `gone` covers and no selector in `kept` still
    /// covers, counting them under [`unwatched`](Self::unwatched). Returns
    /// how many were retired. Selectors that fail to parse as key
    /// expressions cover nothing (`gone`) / keep nothing (`kept`).
    pub fn retire_unwatched(&mut self, gone: &str, kept: &[String]) -> usize {
        use zenoh::key_expr::keyexpr;
        // Borrowed throughout: `keyexpr::new(&str)` validates without
        // allocating, where `KeyExpr::new(String)` builds an `OwnedKeyExpr`.
        // The old form cloned the key *and* built an owned expr for every key
        // in the table on every unwatch — 100k allocations at the default
        // bound (`docs/zero-copy.md`).
        let Ok(gone) = keyexpr::new(gone) else {
            return 0;
        };
        let kept: Vec<&keyexpr> = kept
            .iter()
            .filter_map(|k| keyexpr::new(k.as_str()).ok())
            .collect();
        let doomed: Vec<String> = self
            .keys
            .keys()
            .filter(|key| match keyexpr::new(key.as_str()) {
                Ok(ke) => gone.intersects(ke) && !kept.iter().any(|k| k.intersects(ke)),
                Err(_) => false,
            })
            .cloned()
            .collect();
        for key in &doomed {
            self.keys.remove(key);
        }
        self.unwatched += doomed.len() as u64;
        doomed.len()
    }

    /// Drop the least-recently-seen entries until there is room.
    fn evict(&mut self) {
        let target = self.max_keys - (self.max_keys / EVICT_FRACTION).max(1);
        let mut seen: Vec<(Instant, String)> = self
            .keys
            .iter()
            .map(|(k, s)| (s.last_seen, k.clone()))
            .collect();
        // Oldest first.
        seen.sort_unstable_by_key(|(last_seen, _)| *last_seen);
        for (_, key) in seen.into_iter().take(self.keys.len() - target) {
            self.keys.remove(&key);
            self.evicted += 1;
        }
    }

    /// Record one sample. `now` is injected for deterministic tests;
    /// `latency_us` is the pre-computed skewed latency (#119) — `None` for
    /// an unstamped sample, which is counted, not defaulted.
    pub fn record(
        &mut self,
        key: &str,
        payload_len: usize,
        sn: Option<u32>,
        now: Instant,
        latency_us: Option<i64>,
    ) {
        if let Some(s) = self.keys.get_mut(key) {
            let dt = now.saturating_duration_since(s.last_seen).as_secs_f64();
            if dt > 0.0 {
                let alpha = 1.0 - (-dt / TAU.as_secs_f64()).exp();
                let instant_rate = 1.0 / dt;
                s.rate_hz += alpha * (instant_rate - s.rate_hz);
            }
            s.count += 1;
            s.bytes += payload_len as u64;
            s.last_seen = now;
            if let (Some(prev), Some(cur)) = (s.last_sn, sn)
                && cur > prev + 1
            {
                s.sn_gaps += u64::from(cur - prev - 1);
            }
            s.last_sn = sn;
            match latency_us {
                Some(us) => {
                    if s.lat.len() >= LAT_WINDOW {
                        s.lat.pop_front();
                    }
                    s.lat.push_back(us);
                }
                None => s.unstamped += 1,
            }
        } else {
            if self.keys.len() >= self.max_keys {
                self.evict();
            }
            self.keys.insert(
                key.to_string(),
                KeyStats {
                    count: 1,
                    bytes: payload_len as u64,
                    rate_hz: 0.0,
                    last_seen: now,
                    sn_gaps: 0,
                    unstamped: u64::from(latency_us.is_none()),
                    last_sn: sn,
                    lat: latency_us.into_iter().collect(),
                },
            );
        }
    }

    pub fn get(&self, key: &str) -> Option<&KeyStats> {
        self.keys.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &KeyStats)> {
        self.keys.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Aggregate totals: (samples, bytes, summed EWMA rate).
    pub fn totals(&self) -> (u64, u64, f64) {
        self.keys.values().fold((0, 0, 0.0), |(c, b, r), s| {
            (c + s.count, b + s.bytes, r + s.rate_hz)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An unbounded table is a leak for any observer that runs for hours on a
    /// bus with content-addressed or per-request keys.
    #[test]
    fn the_table_is_bounded() {
        let mut t = StatsTable::with_capacity(100);
        let now = Instant::now();
        for i in 0..1000 {
            t.record(&format!("demo/k{i}"), 4, None, now, None);
        }
        assert!(t.len() <= 100, "len {} exceeds the bound", t.len());
        assert!(t.evicted() > 0);
        // Nothing vanishes silently: every key seen is either present or counted.
        assert_eq!(t.len() as u64 + t.evicted(), 1000);
    }

    /// Eviction is least-recently-seen: a key still publishing must outlive a
    /// key that went quiet, or a live view would drop exactly what it is for.
    #[test]
    fn eviction_drops_the_least_recently_seen() {
        let mut t = StatsTable::with_capacity(10);
        let t0 = Instant::now();

        // Ten keys, oldest first.
        for i in 0..10 {
            t.record(
                &format!("old/k{i}"),
                4,
                None,
                t0 + Duration::from_millis(i),
                None,
            );
        }
        // One of them keeps publishing, much later.
        let fresh = t0 + Duration::from_secs(60);
        t.record("old/k0", 4, None, fresh, None);

        // Now push new keys in, forcing eviction. Each is strictly newer than
        // `old/k0`'s refresh, so there is no tie for "oldest" to break.
        for i in 1..=5 {
            t.record(
                &format!("new/k{i}"),
                4,
                None,
                fresh + Duration::from_millis(i),
                None,
            );
        }

        assert!(
            t.get("old/k0").is_some(),
            "a key that is still publishing must survive"
        );
        assert!(
            t.get("old/k1").is_none(),
            "a key that went quiet should have been evicted first"
        );
    }

    /// #119: the latency window summarises stamped samples and counts
    /// unstamped ones separately — no latency is not zero latency, and a
    /// negative value is the skew evidence, kept.
    #[test]
    fn latency_is_summarised_and_unstamped_is_counted_not_defaulted() {
        let mut t = StatsTable::new();
        let now = Instant::now();
        for us in [1000, -200, 5000, 3000] {
            t.record("k", 4, None, now, Some(us));
        }
        t.record("k", 4, None, now, None);
        let s = t.get("k").unwrap();
        assert_eq!(s.unstamped, 1);
        let lat = s.latency().unwrap();
        assert_eq!(lat.min_us, -200, "negative skew is shown, not clamped");
        assert_eq!(lat.max_us, 5000);
        assert_eq!(lat.samples, 4);
        assert!(lat.median_us >= -200 && lat.median_us <= 5000);

        // Never stamped: no summary, rather than an invented zero.
        t.record("quiet", 4, None, now, None);
        assert!(t.get("quiet").unwrap().latency().is_none());
        assert_eq!(t.get("quiet").unwrap().unstamped, 1);
    }

    /// Updating a known key must never evict — the bound is on distinct keys,
    /// not on samples.
    #[test]
    fn repeated_keys_never_trigger_eviction() {
        let mut t = StatsTable::with_capacity(4);
        let t0 = Instant::now();
        for i in 0..1000 {
            t.record("demo/one", 4, None, t0 + Duration::from_millis(i), None);
        }
        assert_eq!(t.len(), 1);
        assert_eq!(t.evicted(), 0);
        assert_eq!(t.get("demo/one").unwrap().count, 1000);
    }

    /// A degenerate bound must not panic or spin.
    #[test]
    fn a_capacity_of_one_still_works() {
        let mut t = StatsTable::with_capacity(1);
        let now = Instant::now();
        t.record("a", 1, None, now, None);
        t.record("b", 1, None, now, None);
        assert_eq!(t.len(), 1);
        assert_eq!(t.evicted(), 1);
        // Zero is clamped rather than accepted.
        assert_eq!(StatsTable::with_capacity(0).max_keys(), 1);
    }

    #[test]
    fn rates_converge_and_gaps_count() {
        let mut t = StatsTable::new();
        let t0 = Instant::now();
        // 10 Hz for 100 samples: the EWMA converges near 10.
        for i in 0..100u32 {
            t.record(
                "v1/h-a/telemetry/x/m",
                8,
                Some(i),
                t0 + Duration::from_millis(100 * u64::from(i)),
                None,
            );
        }
        let s = t.get("v1/h-a/telemetry/x/m").unwrap();
        assert_eq!(s.count, 100);
        assert_eq!(s.bytes, 800);
        assert!((s.rate_hz - 10.0).abs() < 1.0, "rate {}", s.rate_hz);
        assert_eq!(s.sn_gaps, 0);

        // A sequence jump records the gap.
        t.record(
            "v1/h-a/telemetry/x/m",
            8,
            Some(105),
            t0 + Duration::from_millis(10_100),
            None,
        );
        assert_eq!(t.get("v1/h-a/telemetry/x/m").unwrap().sn_gaps, 5);
    }

    #[test]
    fn totals_aggregate() {
        let mut t = StatsTable::new();
        let now = Instant::now();
        t.record("a", 10, None, now, None);
        t.record("b", 20, None, now, None);
        let (count, bytes, _) = t.totals();
        assert_eq!((count, bytes), (2, 30));
        assert_eq!(t.len(), 2);
    }

    /// Unwatch retirement: covered-by-gone and not-by-kept keys leave the
    /// table, counted separately from bound eviction (O6's third category).
    #[test]
    fn retire_unwatched_respects_remaining_coverage() {
        let mut t = StatsTable::new();
        let now = Instant::now();
        t.record("v1/h-a/telemetry/x/m1", 4, None, now, None);
        t.record("v1/h-a/state/x/health", 4, None, now, None);
        t.record("v1/h-b/telemetry/y/m2", 4, None, now, None);

        // Release the telemetry watch, but keep watching h-a entirely.
        let retired = t.retire_unwatched("v1/*/telemetry/**", &["v1/h-a/**".to_string()]);
        assert_eq!(retired, 1, "only h-b's telemetry loses coverage");
        assert!(
            t.get("v1/h-a/telemetry/x/m1").is_some(),
            "still covered by kept"
        );
        assert!(t.get("v1/h-b/telemetry/y/m2").is_none());
        assert_eq!(t.unwatched(), 1);

        // Release the rest: everything goes, and the ledger adds up.
        let retired = t.retire_unwatched("**", &[]);
        assert_eq!(retired, 2);
        assert_eq!(t.len(), 0);
        assert_eq!(t.unwatched(), 3);
    }

    /// A selector that is not a valid keyexpr covers nothing — no panic, no
    /// accidental mass retirement.
    #[test]
    fn retire_unwatched_tolerates_bad_selectors() {
        let mut t = StatsTable::new();
        t.record("a/b", 1, None, Instant::now(), None);
        assert_eq!(t.retire_unwatched("", &[]), 0);
        assert_eq!(t.len(), 1);
    }
}

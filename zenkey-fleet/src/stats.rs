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
/// (arrival wall-clock − publisher HLC), µs, over the last `LAT_WINDOW`
/// (a private bound) stamped samples.
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

/// Which clock stamped a latency observation — the storage form of
/// [`crate::sub::StampProvenance`], without the stamper's identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StampClass {
    SelfStamped,
    Foreign,
    Unattributable,
}

/// One key's observed latency, kept apart by **who stamped it** (issue #213).
///
/// Three populations, never folded into one median. A publisher-stamped sample
/// measures publisher → observer; a router-stamped one measures that router →
/// observer, which is a different quantity on the same axis. Averaging them
/// produces a number that describes neither, and a fleet where some producers
/// timestamp and some do not would report it without a word.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct LatencyReport {
    /// Samples the publishing session stamped itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_stamped: Option<LatencySummary>,
    /// Samples stamped by another node — commonly a router.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreign: Option<LatencySummary>,
    /// Stamped, but with no `SourceInfo` to compare against: unknown, not
    /// foreign (RFC 09 §5.1 O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unattributable: Option<LatencySummary>,
    /// The distinct stamping nodes seen on this key, rendered. Empty when
    /// every sample was self-stamped — there is no third party to name.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub stampers: Vec<String>,
    /// Stampers beyond the retained bound that were dropped (O6: a bound
    /// reports what it cost).
    #[serde(skip_serializing_if = "is_zero", default)]
    pub stampers_dropped: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl LatencyReport {
    /// Whether anything was observed at all.
    pub fn is_empty(&self) -> bool {
        self.self_stamped.is_none() && self.foreign.is_none() && self.unattributable.is_none()
    }

    /// The populations present, each with the label that says what it
    /// measures. Ordered self → foreign → unattributable.
    pub fn populations(&self) -> Vec<(&'static str, LatencySummary)> {
        [
            ("publisher-stamped", self.self_stamped),
            ("router-stamped", self.foreign),
            ("stamper unknown", self.unattributable),
        ]
        .into_iter()
        .filter_map(|(label, s)| s.map(|s| (label, s)))
        .collect()
    }

    /// The caveat that has to travel with every rendering of these numbers.
    ///
    /// One sentence, in the engine, so the CLI and the GUI cannot drift into
    /// describing the same measurement differently (RFC 09 §5.1 O7).
    pub fn caveat(&self) -> String {
        let clock = match (self.self_stamped.is_some(), self.foreign.is_some()) {
            (true, false) => "the publisher's own HLC",
            (false, true) => "an HLC stamped in transit, not the publisher's",
            (true, true) => "two different clocks, kept apart below",
            (false, false) => "an HLC whose stamper did not identify itself",
        };
        let mut note = format!(
            "arrival wall-clock − {clock}: observed *skewed* latency — it contains \
             clock skew, and negative values are the skew evidence, not an error \
             (RFC 09 §5.1)"
        );
        if !self.stampers.is_empty() {
            note.push_str(&format!("\nstamped by: {}", self.stampers.join(", ")));
            if self.stampers_dropped > 0 {
                note.push_str(&format!(" (+{} more not retained)", self.stampers_dropped));
            }
        }
        note
    }
}

/// How many distinct stampers one key retains. A key sees its publisher and,
/// at most, the routers on its path; more than this is a fleet-shaped
/// question for `doctor`, not a per-key one.
const MAX_STAMPERS: usize = 4;

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
    /// Bounded window of observed skewed latencies, µs, each tagged with who
    /// stamped it — the tag is what keeps the three populations apart (#213).
    lat: VecDeque<(i64, StampClass)>,
    /// Distinct third-party stampers seen, bounded by [`MAX_STAMPERS`].
    stampers: std::collections::BTreeSet<zenoh::time::TimestampId>,
    /// Stampers the bound refused (O6).
    stampers_dropped: u64,
}

impl KeyStats {
    /// The window's distributions, split by who stamped them (#213).
    ///
    /// `None` before any stamped sample — which is not zero latency, and is
    /// why [`KeyStats::unstamped`] is counted beside this rather than folded
    /// into it.
    pub fn latency(&self) -> Option<LatencyReport> {
        if self.lat.is_empty() {
            return None;
        }
        let of = |class: StampClass| {
            summarise(
                self.lat
                    .iter()
                    .filter(|(_, c)| *c == class)
                    .map(|(us, _)| *us),
            )
        };
        Some(LatencyReport {
            self_stamped: of(StampClass::SelfStamped),
            foreign: of(StampClass::Foreign),
            unattributable: of(StampClass::Unattributable),
            stampers: self.stampers.iter().map(|id| id.to_string()).collect(),
            stampers_dropped: self.stampers_dropped,
        })
    }
}

/// Retain a third-party stamper, or count the bound that refused it (O6).
fn note_stamper(
    set: &mut std::collections::BTreeSet<zenoh::time::TimestampId>,
    dropped: &mut u64,
    stamper: Option<zenoh::time::TimestampId>,
) {
    let Some(id) = stamper else { return };
    if set.contains(&id) {
        return;
    }
    if set.len() >= MAX_STAMPERS {
        *dropped += 1;
        return;
    }
    set.insert(id);
}

/// The distribution of one population, or `None` when it is empty.
fn summarise(values: impl Iterator<Item = i64>) -> Option<LatencySummary> {
    let mut sorted: Vec<i64> = values.collect();
    if sorted.is_empty() {
        return None;
    }
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
    /// `latency` is the pre-computed skewed latency (#119) with the class of
    /// clock that produced it (#213) — `None` for an unstamped sample, which
    /// is counted, not defaulted. `stamper` names a third-party stamping node
    /// when there was one.
    pub fn record(
        &mut self,
        key: &str,
        payload_len: usize,
        sn: Option<u32>,
        now: Instant,
        latency: Option<(i64, StampClass)>,
        stamper: Option<zenoh::time::TimestampId>,
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
            match latency {
                Some(observed) => {
                    if s.lat.len() >= LAT_WINDOW {
                        s.lat.pop_front();
                    }
                    s.lat.push_back(observed);
                }
                None => s.unstamped += 1,
            }
            note_stamper(&mut s.stampers, &mut s.stampers_dropped, stamper);
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
                    unstamped: u64::from(latency.is_none()),
                    last_sn: sn,
                    lat: latency.into_iter().collect(),
                    stampers: {
                        let mut set = std::collections::BTreeSet::new();
                        let mut dropped = 0;
                        note_stamper(&mut set, &mut dropped, stamper);
                        set
                    },
                    stampers_dropped: 0,
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
            t.record(&format!("demo/k{i}"), 4, None, now, None, None);
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
                None,
            );
        }
        // One of them keeps publishing, much later.
        let fresh = t0 + Duration::from_secs(60);
        t.record("old/k0", 4, None, fresh, None, None);

        // Now push new keys in, forcing eviction. Each is strictly newer than
        // `old/k0`'s refresh, so there is no tie for "oldest" to break.
        for i in 1..=5 {
            t.record(
                &format!("new/k{i}"),
                4,
                None,
                fresh + Duration::from_millis(i),
                None,
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
            t.record("k", 4, None, now, Some((us, StampClass::SelfStamped)), None);
        }
        t.record("k", 4, None, now, None, None);
        let s = t.get("k").unwrap();
        assert_eq!(s.unstamped, 1);
        let lat = s.latency().unwrap();
        let own = lat
            .self_stamped
            .expect("the publisher stamped these itself");
        assert_eq!(own.min_us, -200, "negative skew is shown, not clamped");
        assert_eq!(own.max_us, 5000);
        assert_eq!(own.samples, 4);
        assert!(own.median_us >= -200 && own.median_us <= 5000);
        assert!(lat.foreign.is_none(), "nothing else stamped anything");
        assert!(lat.stampers.is_empty(), "no third party to name");

        // Never stamped: no summary, rather than an invented zero.
        t.record("quiet", 4, None, now, None, None);
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
            t.record(
                "demo/one",
                4,
                None,
                t0 + Duration::from_millis(i),
                None,
                None,
            );
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
        t.record("a", 1, None, now, None, None);
        t.record("b", 1, None, now, None, None);
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
            None,
        );
        assert_eq!(t.get("v1/h-a/telemetry/x/m").unwrap().sn_gaps, 5);
    }

    #[test]
    fn totals_aggregate() {
        let mut t = StatsTable::new();
        let now = Instant::now();
        t.record("a", 10, None, now, None, None);
        t.record("b", 20, None, now, None, None);
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
        t.record("v1/h-a/telemetry/x/m1", 4, None, now, None, None);
        t.record("v1/h-a/state/x/health", 4, None, now, None, None);
        t.record("v1/h-b/telemetry/y/m2", 4, None, now, None, None);

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
        t.record("a/b", 1, None, Instant::now(), None, None);
        assert_eq!(t.retire_unwatched("", &[]), 0);
        assert_eq!(t.len(), 1);
    }

    /// #213: a publisher-stamped sample and a router-stamped one measure from
    /// different clocks. Averaging them yields a number that describes
    /// neither, so the summary keeps them apart — and names the third party.
    #[test]
    fn two_stampers_are_never_folded_into_one_median() {
        let mut t = StatsTable::new();
        let now = Instant::now();
        let router = zenoh::time::TimestampId::rand();

        // The publisher stamps its own: tight, sub-millisecond.
        for us in [100, 120, 140, 160] {
            t.record("k", 4, None, now, Some((us, StampClass::SelfStamped)), None);
        }
        // A router stamps the rest, much further from us.
        for us in [9000, 9500, 10_000] {
            t.record(
                "k",
                4,
                None,
                now,
                Some((us, StampClass::Foreign)),
                Some(router),
            );
        }

        let lat = t
            .get("k")
            .unwrap()
            .latency()
            .expect("something was stamped");
        let own = lat.self_stamped.expect("the publisher-stamped population");
        let far = lat.foreign.expect("the router-stamped population");
        assert_eq!(own.samples, 4);
        assert_eq!(far.samples, 3);
        assert_eq!(own.max_us, 160);
        assert_eq!(far.min_us, 9000);
        assert!(
            own.median_us < far.median_us,
            "two populations, two medians: {} vs {}",
            own.median_us,
            far.median_us
        );
        assert_eq!(
            lat.stampers,
            vec![router.to_string()],
            "the third-party stamper is named, not averaged away"
        );
        assert!(lat.unattributable.is_none());

        // A sample with no SourceInfo is *unknown*, never "foreign" (O4).
        let orphan = zenoh::time::TimestampId::rand();
        t.record(
            "u",
            4,
            None,
            now,
            Some((7, StampClass::Unattributable)),
            Some(orphan),
        );
        let u = t.get("u").unwrap().latency().unwrap();
        assert!(u.unattributable.is_some());
        assert!(u.foreign.is_none(), "unknown is not foreign");
    }

    /// The stamper set is bounded like everything else an hours-long observer
    /// accumulates, and it reports what the bound cost (O6).
    #[test]
    fn the_stamper_set_is_bounded_and_says_what_it_dropped() {
        let mut t = StatsTable::new();
        let now = Instant::now();
        for _ in 0..(MAX_STAMPERS + 3) {
            t.record(
                "k",
                4,
                None,
                now,
                Some((10, StampClass::Foreign)),
                Some(zenoh::time::TimestampId::rand()),
            );
        }
        let lat = t.get("k").unwrap().latency().unwrap();
        assert_eq!(lat.stampers.len(), MAX_STAMPERS);
        assert_eq!(lat.stampers_dropped, 3, "the bound reports its cost");
    }

    /// The caveat travels with the numbers and names which clock produced
    /// them — the mislabel #213 exists to fix.
    #[test]
    fn the_caveat_names_the_clock_it_measured_from() {
        let self_only = LatencyReport {
            self_stamped: Some(LatencySummary {
                min_us: 1,
                median_us: 2,
                p95_us: 3,
                max_us: 4,
                samples: 4,
            }),
            ..LatencyReport::default()
        };
        assert!(
            self_only.caveat().contains("the publisher's own HLC"),
            "{}",
            self_only.caveat()
        );

        let router_only = LatencyReport {
            foreign: self_only.self_stamped,
            stampers: vec!["abcd".into()],
            ..LatencyReport::default()
        };
        let note = router_only.caveat();
        assert!(
            note.contains("stamped in transit, not the publisher's"),
            "{note}"
        );
        assert!(note.contains("abcd"), "the stamper is named: {note}");

        let both = LatencyReport {
            self_stamped: self_only.self_stamped,
            foreign: self_only.self_stamped,
            ..LatencyReport::default()
        };
        assert!(both.caveat().contains("kept apart"), "{}", both.caveat());
    }
}

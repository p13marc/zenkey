//! Windowed per-key statistics (issues #13/#15): message/byte counters and
//! an exponentially-weighted rate, keyed by wire key. Backs `zenctl topic
//! hz`/`bw`/`echo --rate` and zengui's tree badges.
//!
//! Perf posture (report §14): lookups borrow (`&str` against the `String`
//! keys — no per-sample allocation on the hot hit path); one allocation per
//! *new* key is the floor.

use std::collections::HashMap;
use std::time::{Duration, Instant};

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
    last_sn: Option<u32>,
}

/// The table. Feed it samples; read it per key or in aggregate.
#[derive(Debug, Default)]
pub struct StatsTable {
    keys: HashMap<String, KeyStats>,
}

/// EWMA time constant (~2 s: responsive enough for a UI badge, smooth
/// enough not to flicker); samples older than ~tau contribute e^-1.
const TAU: Duration = Duration::from_secs(2);

impl StatsTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one sample. `now` is injected for deterministic tests.
    pub fn record(&mut self, key: &str, payload_len: usize, sn: Option<u32>, now: Instant) {
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
        } else {
            self.keys.insert(
                key.to_string(),
                KeyStats {
                    count: 1,
                    bytes: payload_len as u64,
                    rate_hz: 0.0,
                    last_seen: now,
                    sn_gaps: 0,
                    last_sn: sn,
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
        );
        assert_eq!(t.get("v1/h-a/telemetry/x/m").unwrap().sn_gaps, 5);
    }

    #[test]
    fn totals_aggregate() {
        let mut t = StatsTable::new();
        let now = Instant::now();
        t.record("a", 10, None, now);
        t.record("b", 20, None, now);
        let (count, bytes, _) = t.totals();
        assert_eq!((count, bytes), (2, 30));
        assert_eq!(t.len(), 2);
    }
}

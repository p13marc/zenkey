//! The rate plane: per-key throughput and the latency populations behind
//! it, kept apart because a stamped and an unstamped sample do not measure
//! the same thing (#119).

use super::asked::Asked;
use serde::Serialize;

/// One key's measured traffic over a `topic hz`/`topic bw` window.
#[derive(Debug, Clone, Serialize)]
pub struct RateRow {
    pub key: String,
    pub count: u64,
    pub bytes: u64,
    /// Source-sequence gaps. `NotAsked` = `--loss` was not asked — the same
    /// gate the report-level `sn_gaps` always had; the row used to serialize
    /// an uncaveated `0` regardless (#238's twin, review finding R3). Even
    /// when asked, zero also means "publishers attach no SourceInfo" — an
    /// observation, not proof of losslessness.
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub sn_gaps: Asked<u64>,
    /// Observed **skewed** latency over the window (#119). Gated on
    /// `--latency` like `unstamped` (#238), but deliberately still `Option`
    /// — the [`Asked`] split's other half: even when asked, it is absent
    /// when **no sample was HLC-stamped**, which is asked-but-absent (not
    /// zero latency, and not "not asked"). Whether the gate was on is what
    /// `unstamped` being `Asked(_)` says. Split by who stamped it (#213):
    /// the three populations measure from different clocks and are never
    /// folded into one median.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency: Option<crate::report::LatencyReport>,
    /// Samples that carried no HLC — the other half of the latency
    /// observation, so it rides the same gate: `NotAsked` = `--latency` was
    /// not asked (R3, matching #238's fix for `latency` itself).
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub unstamped: Asked<u64>,
}

/// The `topic hz` / `topic bw` report (issue #46) — measured counts plus the
/// O6 bound honesty: a bounded [`StatsTable`](crate::model::stats::StatsTable) that retired
/// keys must say so, or the totals silently claim more coverage than they
/// have.
#[derive(Debug, Clone, Serialize)]
pub struct RateReport {
    pub selector: String,
    pub window_s: f64,
    /// Rows are present only for a `--per-key` run, sorted by count
    /// descending.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<RateRow>,
    pub total_count: u64,
    pub total_bytes: u64,
    /// Concrete keys retained by the stats table over the window.
    pub keys: usize,
    /// Keys retired to stay within the table bound (RFC 09 §5.1 O6) — the
    /// totals cover the retained set only.
    pub evicted: u64,
    /// The bound the table ran under.
    pub max_keys: usize,
    /// Total source-sequence gaps (`NotAsked` = `--loss` was not asked).
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub sn_gaps: Asked<u64>,
}

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

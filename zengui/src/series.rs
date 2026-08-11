//! Numeric series for plotting (issue #64) — the whole of the logic, kept out
//! of the drawing code.
//!
//! A canvas is invisible to `iced_test`'s find-by-text, so anything a test
//! needs to pin has to live here as a pure function over plain data (the
//! `view::tree::flatten` precedent). What the canvas then does is turn a
//! `Series` into a path.
//!
//! Two series, from two different sources and two different clocks:
//!
//! - **value** — a numeric leaf of the payload, one point per retained history
//!   entry, on the history ring's arrival order;
//! - **rate** — the engine's EWMA for the key, sampled once per stats tick.
//!
//! Both are `Option<f64>`, and `None` means **gap**: a window with no
//! measurement is drawn as a break, never bridged. Interpolating across a gap
//! invents data, and on a rate series it would specifically invent liveness.
//!
//! Values come from the structural sniff, not from a schema decode: resolving
//! a schema is async and may GET a `describe` on a miss, which must never sit
//! on a render path. The consequence is real and stated on screen — a
//! protobuf or CDR leaf offers no chart until something decodes it cheaply.

use serde_json::Value;

use crate::history::HistoryRing;

/// How many numeric leaves one payload may offer before the list is cut.
///
/// A picker is a list a human reads; a foreign publisher's thousand-field
/// document is not one. What was dropped is reported by
/// [`NumericLeaves::truncated`].
const MAX_LEAVES: usize = 64;

/// How deep the leaf walk descends. Same reasoning as the diff's bound.
const MAX_DEPTH: usize = 16;

/// How many points a series retains. Beyond this the oldest are dropped —
/// the chart is a window, and [`Series::dropped`] says how much fell out of it.
const MAX_POINTS: usize = 600;

/// The numeric leaves of one payload, in traversal order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NumericLeaves {
    /// `(dotted path, value)` — the same path grammar the diff uses.
    pub leaves: Vec<(String, f64)>,
    /// Leaves found past the bound and therefore not listed.
    pub truncated: usize,
}

/// Every numeric leaf of a structural value, addressed by dotted path.
///
/// Booleans are not numbers here on purpose: plotting `true` as 1 turns a
/// state flag into a measurement, which is a different claim.
pub fn numeric_leaves(value: &Value) -> NumericLeaves {
    let mut out = NumericLeaves::default();
    walk(String::new(), value, 0, &mut out);
    out
}

fn walk(path: String, value: &Value, depth: usize, out: &mut NumericLeaves) {
    if depth >= MAX_DEPTH {
        return;
    }
    match value {
        Value::Number(n) => {
            if let Some(f) = n.as_f64()
                && f.is_finite()
            {
                if out.leaves.len() < MAX_LEAVES {
                    out.leaves.push((path, f));
                } else {
                    out.truncated += 1;
                }
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                walk(join(&path, k), v, depth + 1, out);
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                walk(join(&path, &i.to_string()), v, depth + 1, out);
            }
        }
        _ => {}
    }
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

/// Read one dotted path out of a value, if it is a finite number there.
pub fn value_at(value: &Value, path: &str) -> Option<f64> {
    let mut cur = value;
    for chunk in path.split('.') {
        cur = match cur {
            Value::Object(map) => map.get(chunk)?,
            Value::Array(items) => items.get(chunk.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    cur.as_f64().filter(|f| f.is_finite())
}

/// A bounded sequence of measurements, oldest first, with gaps preserved.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Series {
    points: Vec<Option<f64>>,
    dropped: u64,
}

impl Series {
    pub fn new() -> Series {
        Series::default()
    }

    /// Record a measurement.
    pub fn push(&mut self, v: f64) {
        self.push_point(Some(v));
    }

    /// Record that this window had no measurement.
    ///
    /// Not zero, and not the previous value carried forward: both of those are
    /// claims. A gap is drawn as a break in the line.
    pub fn push_gap(&mut self) {
        self.push_point(None);
    }

    fn push_point(&mut self, p: Option<f64>) {
        self.points.push(p);
        if self.points.len() > MAX_POINTS {
            let overflow = self.points.len() - MAX_POINTS;
            self.points.drain(..overflow);
            self.dropped += overflow as u64;
        }
    }

    pub fn points(&self) -> &[Option<f64>] {
        &self.points
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Points that fell out of the window.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Whether any point is a measurement — a series of nothing but gaps has
    /// no chart to draw, and saying "0" for it would be a lie.
    pub fn has_data(&self) -> bool {
        self.points.iter().any(Option::is_some)
    }

    /// The newest measurement, skipping trailing gaps.
    pub fn last(&self) -> Option<f64> {
        self.points.iter().rev().find_map(|p| *p)
    }

    /// `(min, max)` over the measurements, ignoring gaps.
    pub fn bounds(&self) -> Option<(f64, f64)> {
        let mut it = self.points.iter().filter_map(|p| *p);
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v))))
    }

    /// How many windows carried no measurement.
    pub fn gaps(&self) -> usize {
        self.points.iter().filter(|p| p.is_none()).count()
    }
}

/// The value series for one dotted path, over a history ring.
///
/// One point per retained entry, oldest first. A tombstone is a gap — a
/// retired key has no value, and plotting the last one through the deletion
/// would draw a line across the very thing the delete announced.
pub fn value_series(ring: &HistoryRing, path: &str) -> Series {
    let mut series = Series::new();
    // `iter` is newest-first; a series reads the other way.
    for entry in ring.iter().collect::<Vec<_>>().into_iter().rev() {
        match entry.value.as_ref().filter(|_| !entry.is_delete) {
            Some(v) => match value_at(v, path) {
                Some(f) => series.push(f),
                None => series.push_gap(),
            },
            None => series.push_gap(),
        }
    }
    series
}

/// Samples the engine's per-key rate once per stats tick.
///
/// The EWMA in `StatsTable` only moves when a sample lands, so a key that
/// stops publishing keeps its last rate for as long as the process runs.
/// Sampling it blindly would draw a confident flat line for a dead key — the
/// plotted equivalent of treating silence as a verdict. So a tick that did not
/// advance the key's sample count records a **gap**, and the chart shows the
/// traffic stopping.
#[derive(Debug, Clone, Default)]
pub struct RateSampler {
    series: Series,
    last_count: Option<u64>,
}

impl RateSampler {
    pub fn new() -> RateSampler {
        RateSampler::default()
    }

    /// Record one tick's observation: the key's cumulative sample count and
    /// its current EWMA rate, or `None` when the key is not in the table at
    /// all (nothing has ever been seen on it).
    pub fn tick(&mut self, observed: Option<(u64, f64)>) {
        match observed {
            Some((count, rate)) if self.last_count.is_some_and(|last| count > last) => {
                self.series.push(rate);
                self.last_count = Some(count);
            }
            Some((count, rate)) => {
                // The first tick has nothing to compare against: take the
                // reading, since the count arriving at all means traffic.
                if self.last_count.is_none() {
                    self.series.push(rate);
                } else {
                    self.series.push_gap();
                }
                self.last_count = Some(count);
            }
            None => self.series.push_gap(),
        }
    }

    pub fn series(&self) -> &Series {
        &self.series
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numeric_leaves_are_found_by_dotted_path() {
        let l = numeric_leaves(&json!({
            "value": 42.0,
            "unit": "percent",
            "nested": {"depth": 3},
            "xs": [1, 2],
            "up": true,
        }));
        let paths: Vec<&str> = l.leaves.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, ["nested.depth", "value", "xs.0", "xs.1"]);
        assert_eq!(l.leaves[1].1, 42.0);
        assert_eq!(l.truncated, 0);
    }

    /// A boolean is a state, not a measurement: plotting `true` as 1 would
    /// turn a flag into a number nobody published.
    #[test]
    fn booleans_and_strings_are_not_numbers() {
        let l = numeric_leaves(&json!({"up": true, "name": "a", "n": 1}));
        assert_eq!(l.leaves, [("n".to_string(), 1.0)]);
    }

    #[test]
    fn the_leaf_list_is_bounded_and_says_what_it_cut() {
        let mut map = serde_json::Map::new();
        for i in 0..(MAX_LEAVES + 10) {
            map.insert(format!("f{i:03}"), json!(i));
        }
        let l = numeric_leaves(&Value::Object(map));
        assert_eq!(l.leaves.len(), MAX_LEAVES);
        assert_eq!(l.truncated, 10);
    }

    #[test]
    fn value_at_reads_nested_paths_and_refuses_non_numbers() {
        let v = json!({"a": {"b": [7, 8]}, "s": "x"});
        assert_eq!(value_at(&v, "a.b.1"), Some(8.0));
        assert_eq!(value_at(&v, "s"), None);
        assert_eq!(value_at(&v, "a.missing"), None);
        assert_eq!(value_at(&v, "a.b.9"), None);
    }

    #[test]
    fn a_series_keeps_gaps_and_reports_its_shape() {
        let mut s = Series::new();
        s.push(1.0);
        s.push_gap();
        s.push(3.0);
        assert_eq!(s.points(), [Some(1.0), None, Some(3.0)]);
        assert_eq!(s.bounds(), Some((1.0, 3.0)));
        assert_eq!(s.last(), Some(3.0));
        assert_eq!(s.gaps(), 1);
        assert!(s.has_data());

        let mut empty = Series::new();
        empty.push_gap();
        assert!(!empty.has_data(), "gaps alone are not data");
        assert_eq!(empty.bounds(), None);
        assert_eq!(empty.last(), None);
    }

    #[test]
    fn a_series_is_a_bounded_window_that_counts_what_left_it() {
        let mut s = Series::new();
        for i in 0..(MAX_POINTS + 25) {
            s.push(i as f64);
        }
        assert_eq!(s.len(), MAX_POINTS);
        assert_eq!(s.dropped(), 25);
    }

    fn ring_of(samples: &[(&[u8], bool)]) -> HistoryRing {
        let mut ring = HistoryRing::new(64);
        for (payload, is_delete) in samples {
            ring.push(&zenkey_fleet::SampleView {
                key: "k".to_string(),
                payload: zenoh::bytes::ZBytes::from(payload.to_vec()),
                encoding: String::new(),
                kind: if *is_delete {
                    zenoh::sample::SampleKind::Delete
                } else {
                    zenoh::sample::SampleKind::Put
                },
                timestamp: None,
                attachment: None,
                received: std::time::Instant::now(),
            });
        }
        ring
    }

    /// Oldest first, and a tombstone breaks the line rather than carrying the
    /// retired value across the deletion.
    #[test]
    fn a_value_series_reads_oldest_first_and_breaks_at_a_tombstone() {
        let ring = ring_of(&[
            (br#"{"value":1}"#, false),
            (br#"{"value":2}"#, false),
            (b"", true),
            (br#"{"value":9}"#, false),
        ]);
        let s = value_series(&ring, "value");
        assert_eq!(s.points(), [Some(1.0), Some(2.0), None, Some(9.0)]);
    }

    /// A path a payload does not carry is a gap, not a zero, and an opaque
    /// payload is a gap too — neither is a measurement of nothing.
    #[test]
    fn a_missing_path_or_an_opaque_payload_is_a_gap() {
        let ring = ring_of(&[
            (br#"{"value":1}"#, false),
            (br#"{"other":5}"#, false),
            (b"just a plain string", false),
        ]);
        let s = value_series(&ring, "value");
        assert_eq!(s.points(), [Some(1.0), None, None]);
        assert_eq!(s.gaps(), 2);
    }

    /// The EWMA never decays on its own, so a tick with no new sample must be
    /// a gap — otherwise a dead key plots as a healthy flat line.
    #[test]
    fn the_rate_sampler_gaps_when_the_count_did_not_advance() {
        let mut r = RateSampler::new();
        r.tick(Some((10, 5.0))); // first reading
        r.tick(Some((14, 5.0))); // traffic
        r.tick(Some((14, 5.0))); // EWMA unchanged and no new sample: silence
        r.tick(Some((14, 5.0)));
        r.tick(Some((20, 4.0))); // traffic again
        assert_eq!(
            r.series().points(),
            [Some(5.0), Some(5.0), None, None, Some(4.0)]
        );
    }

    /// A key absent from the stats table has never been seen at all — also a
    /// gap, and specifically not a zero rate.
    #[test]
    fn an_unseen_key_records_gaps_not_zeroes() {
        let mut r = RateSampler::new();
        r.tick(None);
        r.tick(None);
        assert!(!r.series().has_data());
        assert_eq!(r.series().gaps(), 2);
    }
}

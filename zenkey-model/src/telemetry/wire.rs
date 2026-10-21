//! The standard schema's types as an owner puts them (§2.3; #819, TB): the
//! Point, its tagged value and the histogram, with serde, so a value of
//! each is the schema's on the wire, in JSON and in CBOR alike.
//!
//! | `telemetry.v1.json` | Here |
//! |---|---|
//! | `telemetry.v1.Point`: `timestamp`, `value`, `labels`, `unit`, `epoch` | [`Point`]: an optional member left out when it is empty, labels in key order |
//! | `telemetry.v1.Value`: `{"type": <kind>, "value": …}`, one branch per kind | [`Value`], internally tagged, the tag spelled as `telemetry.kind` spells it (§2.2) |
//! | `binary`: an array of byte values, in JSON and in CBOR alike | [`Value::Binary`], a `Vec<u8>` serialized as a sequence, never a byte string |
//! | `telemetry.v1.Histogram`: `buckets`, `counts`, `count`, `sum` | [`Histogram`], and [`Histogram::observe`], which keeps §2.8's shape |
//! | `telemetry.v1.Epoch` | [`Point::epoch`], a string |
//!
//! These are session-free, so they live beside the readers: the runtime's
//! writers check what they put through [`super::check`], as any reader
//! would, and fill the epoch an application leaves out (`zenkey::telemetry`).
//! A value of these types is not a conforming sample by construction: a
//! counter Point on a resource declared `gauge`, a histogram over bounds
//! other than the declared ones, or a gauge that is not finite all
//! serialize, and the writer refuses them.
//!
//! ZenSight's `TelemetryPoint` reads as a [`Point`]: its `source` and
//! `metric` are members the schema does not name, which every reader
//! tolerates (core §9.8), and its value is the same tagged union.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{MetricKind, ascending};

/// One measurement of one member (§2.3): the series is the key, never a
/// label (§2.12).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// The measurement time, Unix epoch milliseconds: when the value was
    /// taken, not when it was put (§2.12).
    pub timestamp: i64,
    /// The measured value, tagged with its kind.
    pub value: Value,
    /// Context, never identity (§2.12). Left out when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
    /// The member's unit where the resource declares none (§2.5). A
    /// declared `telemetry.unit` is authoritative, and a Point that
    /// disagrees does not conform (§2.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Its in-band epoch (§2.9). The runtime's writer fills it with the
    /// owner's instance id, or the member's epoch, when it is left out on a
    /// counter or a histogram.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<String>,
}

impl Point {
    /// A Point of `value`, measured at `timestamp_ms` (Unix epoch
    /// milliseconds). `zenkey::telemetry::point` stamps one with the clock
    /// now; this module reads no clock.
    #[must_use]
    pub fn new(timestamp_ms: i64, value: impl Into<Value>) -> Self {
        Self {
            timestamp: timestamp_ms,
            value: value.into(),
            labels: BTreeMap::new(),
            unit: None,
            epoch: None,
        }
    }

    /// With the unit `unit` (§2.5).
    #[must_use]
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    /// With the label `name` = `value`, context (§2.12).
    #[must_use]
    pub fn with_label(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.insert(name.into(), value.into());
        self
    }

    /// With the in-band epoch `epoch` (§2.9). An owner rarely writes it: the
    /// runtime's writer does, and refuses one other than its own.
    #[must_use]
    pub fn with_epoch(mut self, epoch: impl Into<String>) -> Self {
        self.epoch = Some(epoch.into());
        self
    }

    /// The Point's tag (§2.5).
    #[must_use]
    pub fn kind(&self) -> MetricKind {
        self.value.kind()
    }
}

/// A Point's value, internally tagged: `{"type": <kind>, "value": …}`
/// (§2.3), one variant per kind of §2.2, in the schema's order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum Value {
    /// A count from 0 to 2^64−1 that never decreases within one epoch
    /// (§2.7).
    Counter(u64),
    /// A number, which may go up or down. JSON has no NaN or infinity: one
    /// serializes as `null`, which does not conform (§2.6).
    Gauge(f64),
    Text(String),
    Boolean(bool),
    /// Bytes, as an array of byte values in JSON and in CBOR alike.
    Binary(Vec<u8>),
    Histogram(Histogram),
}

impl Value {
    /// The kind its tag names.
    #[must_use]
    pub fn kind(&self) -> MetricKind {
        match self {
            Self::Counter(_) => MetricKind::Counter,
            Self::Gauge(_) => MetricKind::Gauge,
            Self::Text(_) => MetricKind::Text,
            Self::Boolean(_) => MetricKind::Boolean,
            Self::Binary(_) => MetricKind::Binary,
            Self::Histogram(_) => MetricKind::Histogram,
        }
    }
}

impl From<u64> for Value {
    fn from(v: u64) -> Self {
        Self::Counter(v)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Self::Gauge(v)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::Boolean(v)
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::Text(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::Text(v.to_owned())
    }
}

impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Self {
        Self::Binary(v)
    }
}

impl From<Histogram> for Value {
    fn from(v: Histogram) -> Self {
        Self::Histogram(v)
    }
}

/// A fixed-bucket distribution (§2.8), cumulative within one epoch, as
/// ZenSight's `HistogramValue` is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Histogram {
    /// The bounds, strictly ascending and finite, `+Inf` implicit: equal by
    /// value to the resource's `telemetry.buckets`.
    pub buckets: Vec<f64>,
    /// A count per bucket, not cumulative, one more than the bounds, the
    /// overflow last.
    pub counts: Vec<u64>,
    /// The sum of `counts`.
    pub count: u64,
    /// The sum of every value observed, which may be negative.
    pub sum: f64,
}

impl Histogram {
    /// An empty histogram over `bounds`, the declared `telemetry.buckets`
    /// (the declaration's [`super::Declared::buckets`]). `None` unless they
    /// are a valid set of bounds (§2.2): non-empty, finite, strictly
    /// ascending.
    #[must_use]
    pub fn new(bounds: &[f64]) -> Option<Self> {
        let valid = !bounds.is_empty() && bounds.iter().all(|b| b.is_finite()) && ascending(bounds);
        valid.then(|| Self {
            buckets: bounds.to_vec(),
            counts: vec![0; bounds.len() + 1],
            count: 0,
            sum: 0.0,
        })
    }

    /// The bucket a value falls in (§2.8): the first bound at or above it,
    /// `(buckets[i-1], buckets[i]]`, upper-inclusive; past the last bound,
    /// the overflow.
    #[must_use]
    pub fn bucket_of(&self, v: f64) -> usize {
        self.buckets.partition_point(|b| *b < v)
    }

    /// Records one observation: its bucket's count and `count` up by one,
    /// `v` added to `sum`. Refused, `false` and nothing changed, for a
    /// value that is not finite (it has no bucket), and for one that would
    /// take a count past 2^64−1 or the sum to an infinity, either of which
    /// would make the histogram one no reader reads (§2.6).
    pub fn observe(&mut self, v: f64) -> bool {
        if !v.is_finite() {
            return false;
        }
        let i = self.bucket_of(v);
        let sum = self.sum + v;
        let (Some(c), Some(n)) = (
            self.counts.get(i).and_then(|c| c.checked_add(1)),
            self.count.checked_add(1),
        ) else {
            return false;
        };
        if !sum.is_finite() {
            return false;
        }
        self.counts[i] = c;
        self.count = n;
        self.sum = sum;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring::Kind;
    use crate::telemetry::{
        BUCKETS, Conformance, Datum, Declaration, KIND, PUBLISHED, Payload, Problem, TypeView,
        UNIT, check, declaration,
    };
    use serde_json::{Value as Json, json};

    fn point_decl(annotations: &[(&str, Json)]) -> Declaration {
        let ty = TypeView::Json {
            name: crate::telemetry::POINT,
            schema: PUBLISHED[1],
            root: None,
        };
        let a = annotations
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect();
        declaration(Kind::Stream, &a, Some(&ty), None)
    }

    fn conformance(d: &Declaration, datum: &Datum) -> Conformance {
        check(
            d,
            &Payload::Decoded {
                value: datum,
                attachment: None,
            },
        )
        .conformance
    }

    fn cbor(p: &Point) -> Vec<u8> {
        let mut out = Vec::new();
        ciborium::into_writer(p, &mut out).unwrap();
        out
    }

    #[test]
    fn a_point_is_the_schemas_on_the_wire() {
        let p = Point::new(1_760_000_000_123, 42u64)
            .with_unit("By")
            .with_label("oid", "1.3.6")
            .with_epoch("00000000000000aa");
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"timestamp":1760000000123,"value":{"type":"counter","value":42},"labels":{"oid":"1.3.6"},"unit":"By","epoch":"00000000000000aa"}"#
        );
        let bare = Point::new(5, Value::Gauge(-70.5));
        assert_eq!(
            serde_json::to_string(&bare).unwrap(),
            r#"{"timestamp":5,"value":{"type":"gauge","value":-70.5}}"#,
            "an empty member is left out"
        );
        // CBOR carries the same data item: a map of the same members, the
        // binary as an array of integers, never a byte string.
        for p in [
            p,
            bare,
            Point::new(1, vec![0u8, 255]),
            Point::new(1, true),
            Point::new(1, "up"),
        ] {
            let bytes = cbor(&p);
            let item: ciborium::Value = ciborium::from_reader(bytes.as_slice()).unwrap();
            let as_json: Json = item.deserialized().unwrap();
            assert_eq!(as_json, serde_json::to_value(&p).unwrap(), "{p:?}");
            let back: Point = ciborium::from_reader(bytes.as_slice()).unwrap();
            assert_eq!(back, p);
            let d = point_decl(&[]);
            let datum = Datum::from_cbor_bytes(&bytes).unwrap();
            assert_eq!(conformance(&d, &datum), Conformance::Conforms, "{p:?}");
            let datum = Datum::from_json_bytes(&serde_json::to_vec(&p).unwrap()).unwrap();
            assert_eq!(conformance(&d, &datum), Conformance::Conforms, "{p:?}");
        }
        let binary = serde_json::to_value(Point::new(1, vec![7u8])).unwrap();
        assert_eq!(binary["value"], json!({"type": "binary", "value": [7]}));
    }

    #[test]
    fn zensights_point_reads_as_one() {
        let z = json!({"timestamp": 3, "source": "h1", "metric": "cpu/0", "value":
            {"type": "gauge", "value": 1.5}, "labels": {"unit": "%"}});
        let p: Point = serde_json::from_value(z).unwrap();
        assert_eq!(p.value, Value::Gauge(1.5));
        assert_eq!(p.unit, None, "a label named unit is context (§2.5)");
    }

    #[test]
    fn a_histogram_observes_into_its_bounds_upper_inclusive() {
        let mut h = Histogram::new(&[0.005, 0.01, 0.025]).unwrap();
        for v in [0.001, 0.005, 0.007, 0.01, 0.02, 0.5, -3.0] {
            assert!(h.observe(v));
        }
        // (−∞,0.005] (0.005,0.01] (0.01,0.025] (0.025,+Inf)
        assert_eq!(h.counts, vec![3, 2, 1, 1]);
        assert_eq!(h.count, 7);
        assert!((h.sum - (0.543 - 3.0)).abs() < 1e-12);
        assert!(!h.observe(f64::NAN) && !h.observe(f64::INFINITY));
        assert!(h.observe(f64::MAX));
        assert!(!h.observe(f64::MAX), "no infinite sum");
        assert_eq!(h.count, 8, "the refused observation counts nothing");
        let d = point_decl(&[
            (KIND, json!("histogram")),
            (UNIT, json!("s")),
            (BUCKETS, json!([0.005, 0.01, 0.025])),
        ]);
        let p = Point::new(1, h.clone()).with_unit("s");
        let datum = Datum::from_json_bytes(&serde_json::to_vec(&p).unwrap()).unwrap();
        assert_eq!(conformance(&d, &datum), Conformance::Conforms);
        let other = Point::new(1, Histogram::new(&[0.005, 0.01, 0.05]).unwrap());
        let datum = Datum::from_json_bytes(&serde_json::to_vec(&other).unwrap()).unwrap();
        assert_eq!(
            conformance(&d, &datum),
            Conformance::DoesNot(vec![Problem::BoundsDiffer])
        );
        assert_eq!(Histogram::new(&[]), None);
        assert_eq!(Histogram::new(&[1.0, 1.0]), None);
        assert_eq!(Histogram::new(&[1.0, f64::INFINITY]), None);
        let mut full = Histogram::new(&[1.0]).unwrap();
        full.counts[0] = u64::MAX;
        full.count = u64::MAX;
        assert!(!full.observe(0.5), "a count past 2^64-1 is refused");
        assert_eq!(full.counts[0], u64::MAX);
    }
}

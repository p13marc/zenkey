//! `telemetry.v1` in zengui (#819, TD): a selected key's metric member, read
//! the way the profile's reader reads one — its kind and unit (§2.5), each
//! sample's conformance (§2.6), the value a chart plots, and a counter's
//! rate over each pair of one epoch (§2.10, §2.11).
//!
//! **Session-free, and every verdict the fleet's.** [`read`] folds a slot's
//! history ring, oldest first, through [`zenkey_fleet::TelemetryBook`], the
//! session-free book `zenctl telemetry` folds a live reading into, and keeps
//! what the book said of each entry: its conforming value, and the pair it
//! closed. No kind, unit, conformance, continuity or rate is computed here.
//! Two things are decided here, and both are about drawing:
//!
//! - **The plotted leaf** of a conforming sample. A counter's or a gauge's
//!   value is plotted: a Point's `value.value`, or the bare value. A boolean
//!   is plotted as 1 or 0. A histogram is plotted as its **mean**,
//!   `sum / count`: the one number of a histogram that carries its member's
//!   unit (`count` counts observations, in no unit of the member's), and a
//!   gap while `count` is 0, since absent is not zero (§2.12). Text and
//!   bytes are not plotted.
//! - **A counter's rate**, one point per ring entry: the §2.11 rate of the
//!   pair the entry closed, else a gap. There is a gap at a reset and at a
//!   decrease, which have no rate, at a sample that did not conform, and at
//!   the window's first sample. A rate is never carried across a reset, and
//!   never restarted from zero.
//!
//! **No presence.** A reader may judge an epoch by presence only when it
//! folds the tokens' moves and the samples in the one order it received
//! them (§2.9, 0.3). zengui's presence rides a monitor of its own, beside
//! the data pump, so the two arrive in no one order. The book is opened
//! reading no presence, as across a face (§2.14), and an epoch here is
//! in-band or assumed. A decrease is a finding only within an in-band
//! epoch; with none, it reads as a reset (§2.10).
//!
//! **The window is the ring's.** The book is rebuilt from the retained
//! entries, so the charts and the pairs cover exactly what the timeline
//! holds. The oldest retained sample closes no pair, whatever came before
//! it, and a state member's merge (§2.10) runs over the window.

use std::sync::Arc;
use std::time::Instant;

use zenkey_fleet::model::telemetry::{invalid_words, problem_words};
use zenkey_fleet::{MetricResource, MetricSample, MetricVia, PairTally, Resolved, TelemetryBook};
use zenkey_model::telemetry::{
    Basis, Carrier, Continuity, Declaration, Measure, MetricKind, NoRate, Problem, Source, Unasked,
    Via,
};

use crate::history::HistoryRing;
use crate::series::Series;

/// How many decreases a read keeps for the Inspector, the latest. Each is
/// counted in the tally whatever this bound keeps.
pub const DECREASES_KEPT: usize = 4;

/// The root path of a bare value, as the field observation names it
/// (`zenkey_fleet::judge::field::ROOT_PATH`).
pub const ROOT_PATH: &str = "$";

/// A chart axis's unit, or why it has none (§2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unit {
    /// The member's unit, and where it came from: the resource's
    /// annotation, or the Point.
    Known(String, Source),
    /// A metric with no unit: none declared, and no Point carried one.
    /// Unknown, never dimensionless (§2.5).
    Unknown,
    /// The resource's declaration is invalid, so its unit is not read
    /// (§2.2).
    Unobservable,
    /// The key resolves to a resource that is no `telemetry.v1` metric
    /// (§2.1): nothing declares a unit for its leaves.
    NotAMetric,
    /// No contract resolves the key.
    Unresolved,
}

impl Unit {
    /// The unit a caption appends, when there is one.
    pub fn unit(&self) -> Option<&str> {
        match self {
            Unit::Known(u, _) => Some(u),
            _ => None,
        }
    }

    /// The Inspector's UNIT field.
    pub fn words(&self) -> String {
        match self {
            Unit::Known(u, Source::Declared) => format!("{u} · declared"),
            Unit::Known(u, Source::Point) => format!("{u} · from the Point"),
            Unit::Unknown => "unknown (§2.5)".to_string(),
            Unit::Unobservable => "unobservable (§2.2)".to_string(),
            Unit::NotAMetric => "none · not a telemetry.v1 metric".to_string(),
            Unit::Unresolved => "none · no contract resolves the key".to_string(),
        }
    }

    /// Why a chart's axis carries no unit, in a sentence; `None` when it
    /// carries one.
    pub fn why_none(&self) -> Option<&'static str> {
        Some(match self {
            Unit::Known(..) => return None,
            Unit::Unknown => {
                "unit unknown — no telemetry.unit is declared and no Point carried one \
                 (telemetry.v1 §2.5): never read as dimensionless"
            }
            Unit::Unobservable => {
                "unit unobservable — the resource's telemetry.v1 declaration is invalid \
                 (§2.2), so neither its unit nor its kind is read"
            }
            Unit::NotAMetric => {
                "no unit — this resource is no telemetry.v1 metric (§2.1), so nothing \
                 declares one for its leaves, and the axis is not guessed"
            }
            Unit::Unresolved => {
                "no unit — no contract resolves this key, so nothing declares one, and \
                 the axis is not guessed"
            }
        })
    }
}

/// The unit of a key's chart (§2.5): its metric's, when it has one, or why
/// there is none.
pub fn unit_of(resolved: Option<&Resolved>, metric: Option<&MetricRead>) -> Unit {
    match (resolved, metric) {
        (_, Some(m)) => m.unit(),
        (Some(_), None) => Unit::NotAMetric,
        (None, _) => Unit::Unresolved,
    }
}

/// One decrease within an established epoch: the finding (§2.10).
#[derive(Debug, Clone, PartialEq)]
pub struct Decrease {
    /// The history entry that closed the pair.
    pub seq: u64,
    pub earlier: Measure,
    pub later: Measure,
    /// How the epoch was established. `Presence` cannot arise here today
    /// (the module doc), and is worded with §2.9's residual risk if it does.
    pub via: Via,
}

/// The resets a read found, by how the epoch was had (§2.10).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Resets {
    /// Another epoch, established in-band: the samples' epochs differ.
    pub in_band: u64,
    /// Another epoch, established by presence.
    pub by_presence: u64,
    /// A decrease where the epoch is unobservable, read as a reset.
    pub assumed: u64,
}

impl Resets {
    pub fn total(&self) -> u64 {
        self.in_band + self.by_presence + self.assumed
    }
}

/// Why pairs of counters gave no rate (§2.11).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoRates {
    pub reset: u64,
    pub decreased: u64,
    /// The elapsed time was 0 or less.
    pub elapsed: u64,
}

/// What the window's history says of one metric member: see the module doc.
#[derive(Debug, Clone)]
pub struct MetricRead {
    /// What the read was made of: a read over the same window and the same
    /// resolution is the same read ([`Window`]).
    pub window: Window,
    /// The resource's declaration (§2.2): `declared`, or `invalid` with its
    /// reasons.
    pub declaration: Declaration,
    /// The member's kind, as its latest sample gave it, else the declared
    /// one; `None` when unobservable (§2.5).
    pub kind: Option<(MetricKind, Source)>,
    /// The member's unit, as its latest sample gave it, else the declared
    /// one; `None` is unknown, never dimensionless (§2.5).
    pub unit: Option<(String, Source)>,
    /// Values read, a delete not counted.
    pub samples: u64,
    pub conforming: u64,
    pub does_not: u64,
    /// Unobservable samples (§2.6): undecodable bytes.
    pub unobservable: u64,
    pub unobservable_why: Option<Unasked>,
    pub deletes: u64,
    /// The reasons of the latest sample that did not conform (§2.6).
    pub last_does_not: Option<Vec<Problem>>,
    /// Whether the window's newest value conformed: the reasons above are
    /// then history, not the member's current word.
    pub newest_conforms: bool,
    /// The plotted leaf, one point per ring entry (the module doc).
    pub value: Series,
    /// A counter's §2.11 rate, one point per ring entry; `None` for any
    /// other kind.
    pub rate: Option<Series>,
    /// The latest rate's unit: the counter's followed by `/s`, or none when
    /// the counter's is unknown (§2.11).
    pub rate_unit: Option<String>,
    /// The time base the latest pair was timed by: `timestamp`, `stamp` or
    /// `arrival` (§2.11).
    pub rate_base: Option<&'static str>,
    pub no_rate: NoRates,
    /// Every pair, as the book tallied it (§2.10).
    pub tally: PairTally,
    pub resets: Resets,
    /// The latest decreases within an established epoch, at most
    /// [`DECREASES_KEPT`].
    pub decreases: Vec<Decrease>,
}

/// What a read was made of: the ring's extent and the resource it resolved
/// to. Two reads of one window are equal, so a tick that brought the key
/// nothing reuses the last read rather than decoding the ring again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub oldest: Option<u64>,
    pub newest: Option<u64>,
    pub fingerprint: String,
    pub index: usize,
}

impl Window {
    pub fn of(resolved: &Resolved, ring: &HistoryRing) -> Window {
        Window {
            oldest: ring.iter().next_back().map(|e| e.seq),
            newest: ring.newest().map(|e| e.seq),
            fingerprint: resolved.revision.fingerprint().to_string(),
            index: resolved.index(),
        }
    }
}

impl MetricRead {
    /// Whether the declaration is valid, so the member is read at all.
    pub fn declared(&self) -> bool {
        matches!(self.declaration, Declaration::Declared(_))
    }

    /// The carrier, for a valid declaration.
    pub fn carrier(&self) -> Option<Carrier> {
        self.declaration.declared().map(|d| d.carrier)
    }

    /// The declaration's reasons in the fleet's words, when it is invalid.
    pub fn invalid(&self) -> Option<String> {
        match &self.declaration {
            Declaration::Invalid(why) => Some(invalid_words(why)),
            _ => None,
        }
    }

    /// The member's unit as a chart's axis carries it (§2.5).
    pub fn unit(&self) -> Unit {
        match (&self.unit, self.declared()) {
            (Some((u, s)), _) => Unit::Known(u.clone(), *s),
            (None, true) => Unit::Unknown,
            (None, false) => Unit::Unobservable,
        }
    }

    /// The Inspector's METRIC KIND field.
    pub fn kind_words(&self) -> String {
        match (&self.kind, self.declared(), self.carrier()) {
            (Some((k, Source::Declared)), ..) => format!("{k} · declared"),
            (Some((k, Source::Point)), ..) => format!("{k} · from the Point"),
            (None, false, _) => "unobservable (§2.2)".to_string(),
            (None, true, Some(Carrier::Point)) if self.conforming == 0 => {
                "the Point's own · none read yet".to_string()
            }
            (None, true, _) => "unobservable (§2.5)".to_string(),
        }
    }

    /// What the value chart plots, as its caption's label; `None` for a
    /// kind that is not plotted (text, bytes) or an unobservable one.
    pub fn plotted_label(&self) -> Option<&'static str> {
        Some(match self.kind?.0 {
            MetricKind::Counter => "counter",
            MetricKind::Gauge => "gauge",
            MetricKind::Boolean => "boolean (1 = true, 0 = false)",
            MetricKind::Histogram => "histogram mean (sum / count)",
            MetricKind::Text | MetricKind::Binary => return None,
        })
    }

    /// Where the plotted leaf comes from, in words.
    pub fn plotted_from(&self) -> &'static str {
        let histogram = self.kind.is_some_and(|(k, _)| k == MetricKind::Histogram);
        match (self.carrier(), histogram) {
            (Some(Carrier::Point), false) => "the Point's value.value",
            (Some(Carrier::Point), true) => {
                "the Point's value.value: its sum over its count, the one number of a \
                 histogram in its member's unit"
            }
            (_, false) => "the bare value",
            (_, true) => {
                "the bare histogram: its sum over its count, the one number of a \
                 histogram in its member's unit"
            }
        }
    }

    /// The unit a field path carries, when the path is the member's value
    /// (`value.value` of a Point, the root of a bare value) or a histogram's
    /// `sum`; `None` for any other path, which carries none of the
    /// member's (a Point's `timestamp`, a histogram's counts).
    pub fn path_unit(&self, path: &str) -> Option<Unit> {
        let kind = self.kind?.0;
        let leaf = match (self.carrier()?, kind) {
            (Carrier::Point, MetricKind::Counter | MetricKind::Gauge) => "value.value",
            (Carrier::Point, MetricKind::Histogram) => "value.value.sum",
            (Carrier::Bare, MetricKind::Counter | MetricKind::Gauge) => ROOT_PATH,
            (Carrier::Bare, MetricKind::Histogram) => "sum",
            _ => return None,
        };
        (path == leaf).then(|| self.unit())
    }

    /// The latest sample's reasons in the fleet's words (§2.6).
    pub fn reasons(&self) -> Vec<String> {
        self.last_does_not
            .iter()
            .flatten()
            .map(|p| problem_words(*p))
            .collect()
    }
}

/// The value a chart plots for one conforming sample (the module doc).
pub fn plotted(m: &Measure) -> Option<f64> {
    match m {
        // Plotted as the nearest double: a chart's resolution, not the
        // count, which the Inspector reads exactly.
        Measure::Counter(n) => Some(*n as f64),
        Measure::Gauge(f) => Some(*f),
        Measure::Boolean(b) => Some(if *b { 1.0 } else { 0.0 }),
        Measure::Histogram(h) => (h.count > 0).then(|| h.sum / h.count as f64),
        Measure::Text(_) | Measure::Binary(_) => None,
    }
}

/// A measure in a few characters, for a finding's subject.
pub fn measure_words(m: &Measure) -> String {
    match m {
        Measure::Counter(n) => n.to_string(),
        Measure::Gauge(f) => f.to_string(),
        Measure::Boolean(b) => b.to_string(),
        Measure::Histogram(h) => format!("count {}", h.count),
        Measure::Text(s) => format!("{s:?}"),
        Measure::Binary(b) => format!("{} bytes", b.len()),
    }
}

/// `resolved`'s metric member over `ring` (the module doc): `None` when the
/// resource is no metric resource (§2.1). An invalid declaration is read
/// as one, and nothing of its samples is (§2.2).
pub fn read(resolved: &Resolved, ring: &HistoryRing) -> Option<MetricRead> {
    let metric = Arc::new(MetricResource::of(
        &resolved.address,
        &resolved.revision,
        resolved.index(),
    )?);
    let declaration = metric.ty.declaration.clone();
    let declared = declaration.declared().cloned();
    let mut out = MetricRead {
        window: Window::of(resolved, ring),
        kind: declared
            .as_ref()
            .and_then(|d| d.kind)
            .map(|k| (k, Source::Declared)),
        unit: declared
            .as_ref()
            .and_then(|d| d.unit.clone())
            .map(|u| (u, Source::Declared)),
        declaration,
        samples: 0,
        conforming: 0,
        does_not: 0,
        unobservable: 0,
        unobservable_why: None,
        deletes: 0,
        last_does_not: None,
        newest_conforms: false,
        value: Series::new(),
        rate: None,
        rate_unit: None,
        rate_base: None,
        no_rate: NoRates::default(),
        tally: PairTally::default(),
        resets: Resets::default(),
        decreases: Vec::new(),
    };
    if declared.is_none() {
        return Some(out);
    }
    // Arrivals count from the window's oldest, so every one is at or after
    // the book's origin.
    let t0 = ring
        .iter()
        .map(|e| e.received)
        .min()
        .unwrap_or_else(Instant::now);
    let mut book = TelemetryBook::new(None, t0);
    let key = resolved.key.clone();
    let mut rate = Series::new();
    // Oldest first: the ring iterates newest first.
    for entry in ring.iter().rev() {
        let before = book.member(&key).map_or(0, |m| m.conforming);
        let seen = book.sample(
            &metric,
            MetricSample {
                key: key.clone(),
                delete: entry.is_delete,
                payload: entry.payload.to_bytes().into_owned(),
                attachment: entry.attachment.as_ref().map(|a| a.to_bytes().into_owned()),
                stamp: entry.stamp,
                at: entry.received,
                via: MetricVia::Subscription,
            },
        );
        let member = book.member(&key);
        let conformed = member.is_some_and(|m| m.conforming > before);
        if !entry.is_delete {
            out.newest_conforms = conformed;
        }
        match member
            .filter(|_| conformed)
            .and_then(|m| m.latest.as_ref())
            .and_then(|r| plotted(&r.value))
        {
            Some(v) => out.value.push(v),
            None => out.value.push_gap(),
        }
        let Some(seen) = seen else {
            rate.push_gap();
            continue;
        };
        match &seen.pair.rate {
            Ok(r) => {
                rate.push(r.per_s);
                out.rate_unit = r.unit.clone();
                out.rate_base = Some(seen.base);
            }
            Err(why) => {
                rate.push_gap();
                match why {
                    NoRate::Reset => out.no_rate.reset += 1,
                    NoRate::Decreased => out.no_rate.decreased += 1,
                    NoRate::Elapsed => out.no_rate.elapsed += 1,
                    NoRate::Kind => {}
                }
            }
        }
        match (seen.pair.continuity, seen.pair.basis) {
            (Continuity::Reset, Some(Basis::Established(Via::InBand))) => out.resets.in_band += 1,
            (Continuity::Reset, Some(Basis::Established(Via::Presence))) => {
                out.resets.by_presence += 1;
            }
            (Continuity::Reset, _) => out.resets.assumed += 1,
            (Continuity::Decreased, Some(Basis::Established(via))) => {
                if out.decreases.len() == DECREASES_KEPT {
                    out.decreases.remove(0);
                }
                out.decreases.push(Decrease {
                    seq: entry.seq,
                    earlier: seen.earlier.clone(),
                    later: seen.later.clone(),
                    via,
                });
            }
            _ => {}
        }
    }
    if let Some(m) = book.member(&key) {
        if m.kind.is_some() {
            out.kind = m.kind;
        }
        out.unit = m.unit.clone();
        out.samples = m.samples;
        out.conforming = m.conforming;
        out.does_not = m.does_not;
        out.unobservable = m.unobservable;
        out.unobservable_why = m.unobservable_why;
        out.deletes = m.deletes;
        out.last_does_not = m.last_does_not.clone();
        out.tally = m.tally;
    }
    if out.kind.is_some_and(|(k, _)| k == MetricKind::Counter) {
        out.rate = Some(rate);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meter() -> Arc<zenkey_fleet::ContractSet> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zenkey/tests/contracts/meter.v1.toml");
        let (set, problems) = zenkey_fleet::ContractSet::load_path(&path);
        assert!(problems.is_empty(), "{problems:?}");
        Arc::new(set)
    }

    fn resolve(set: &zenkey_fleet::ContractSet, key: &str) -> Resolved {
        zenkey_fleet::Lens::new("", None, set)
            .offline(set)
            .resolve(key)
            .resolved
            .unwrap_or_else(|| panic!("{key} resolves"))
    }

    fn ring(samples: &[(&[u8], Option<&[u8]>)]) -> HistoryRing {
        let t0 = Instant::now();
        let mut ring = HistoryRing::new(64);
        for (i, (payload, attachment)) in samples.iter().enumerate() {
            ring.push(&zenkey_fleet::SampleView {
                key: "k".into(),
                payload: zenoh::bytes::ZBytes::from(payload.to_vec()),
                encoding: "application/json".into(),
                kind: zenoh::sample::SampleKind::Put,
                timestamp: None,
                stamped_by: None,
                attachment: attachment.map(|a| zenoh::bytes::ZBytes::from(a.to_vec())),
                priority: zenoh::qos::Priority::DEFAULT,
                congestion_control: zenoh::qos::CongestionControl::DEFAULT,
                reliability: zenoh::qos::Reliability::DEFAULT,
                express: false,
                source: None,
                received: t0 + std::time::Duration::from_millis(500 * i as u64),
            });
        }
        ring
    }

    /// A Point counter's plotted leaf is `value.value`, never `timestamp`
    /// (the first leaf a structural walk meets); its rate is §2.11's, timed
    /// by the Points' timestamps, in the declared unit and `/s`; and a new
    /// in-band epoch is a reset, a gap with no rate across it.
    #[test]
    fn a_point_counter_plots_its_value_and_rates_within_one_epoch() {
        let set = meter();
        let r = resolve(&set, "zk2/lab/meter/meter.v1/stream/frames");
        let point = |ts: i64, n: u64, epoch: &str| {
            serde_json::to_vec(&serde_json::json!({
                "timestamp": ts, "value": {"type": "counter", "value": n}, "epoch": epoch,
            }))
            .unwrap()
        };
        let (a, b, c, d) = (
            point(1_000, 10, "e1"),
            point(2_000, 30, "e1"),
            point(3_000, 2, "e2"),
            point(4_000, 6, "e2"),
        );
        let m = read(&r, &ring(&[(&a, None), (&b, None), (&c, None), (&d, None)])).unwrap();
        assert_eq!(m.kind, Some((MetricKind::Counter, Source::Declared)));
        assert_eq!(m.unit(), Unit::Known("1".into(), Source::Declared));
        assert_eq!(
            m.value.points(),
            [Some(10.0), Some(30.0), Some(2.0), Some(6.0)]
        );
        let rate = m.rate.as_ref().expect("a counter has a rate series");
        assert_eq!(rate.points(), [None, Some(20.0), None, Some(4.0)]);
        assert_eq!(m.rate_unit.as_deref(), Some("1/s"));
        assert_eq!(m.rate_base, Some("timestamp"));
        assert_eq!(m.resets.in_band, 1);
        assert_eq!(m.no_rate.reset, 1);
        assert!(m.decreases.is_empty());
        assert_eq!(m.plotted_label(), Some("counter"));
        assert_eq!(m.path_unit("value.value"), Some(m.unit()));
        assert_eq!(m.path_unit("timestamp"), None);
    }

    /// A bare counter's decrease within its in-band epoch (the Epoch
    /// attachment) is the finding, with no rate; without an epoch the same
    /// decrease reads as a reset, assumed, and the unit is unknown.
    #[test]
    fn a_decrease_is_a_finding_only_within_an_in_band_epoch() {
        let set = meter();
        let bytes = resolve(&set, "zk2/lab/meter/meter.v1/stream/bytes");
        let m = read(
            &bytes,
            &ring(&[
                (b"100", Some(br#""e1""#)),
                (b"40", Some(br#""e1""#)),
                (b"90", Some(br#""e1""#)),
            ]),
        )
        .unwrap();
        assert_eq!(m.decreases.len(), 1);
        assert_eq!(m.decreases[0].via, Via::InBand);
        assert_eq!(m.no_rate.decreased, 1);
        assert_eq!(
            m.rate.as_ref().unwrap().points(),
            [None, None, Some(100.0)],
            "50 By over 500 ms of arrival"
        );
        assert_eq!(m.rate_unit.as_deref(), Some("By/s"));
        assert_eq!(m.rate_base, Some("arrival"));

        let drops = resolve(&set, "zk2/lab/meter/meter.v1/stream/drops");
        let m = read(&drops, &ring(&[(b"100", None), (b"40", None)])).unwrap();
        assert!(m.decreases.is_empty(), "never a finding without an epoch");
        assert_eq!(m.resets.assumed, 1);
        assert_eq!(m.unit(), Unit::Unknown);
        assert_eq!(m.unit().words(), "unknown (§2.5)");
        assert_eq!(m.path_unit(ROOT_PATH), Some(Unit::Unknown));
    }

    /// A histogram plots its mean, and a gap while it holds nothing; a
    /// Point whose tag disagrees with its declaration does not conform, is
    /// not plotted, and says why in the fleet's words.
    #[test]
    fn a_histogram_plots_its_mean_and_a_disagreeing_tag_is_said() {
        let set = meter();
        let r = resolve(&set, "zk2/lab/meter/meter.v1/stream/latency");
        let hist = |count: u64, sum: f64, counts: [u64; 4]| {
            serde_json::to_vec(&serde_json::json!({
                "timestamp": 1_000, "unit": "s",
                "value": {"type": "histogram", "value": {
                    "buckets": [0.005, 0.01, 0.025], "counts": counts,
                    "count": count, "sum": sum}},
            }))
            .unwrap()
        };
        let empty = hist(0, 0.0, [0, 0, 0, 0]);
        let full = hist(4, 0.04, [1, 1, 1, 1]);
        let gauge = serde_json::to_vec(&serde_json::json!({
            "timestamp": 2_000, "value": {"type": "gauge", "value": 0.2},
        }))
        .unwrap();
        let m = read(&r, &ring(&[(&empty, None), (&full, None), (&gauge, None)])).unwrap();
        assert_eq!(m.value.points(), [None, Some(0.01), None]);
        assert!(m.rate.is_none(), "no rate of a histogram at this version");
        assert_eq!(m.does_not, 1);
        assert!(!m.newest_conforms);
        assert_eq!(
            m.reasons(),
            ["telemetry.v1 §2.6 tag_disagrees: the Point's tag differs from the declared kind"]
        );
        assert_eq!(m.plotted_label(), Some("histogram mean (sum / count)"));
        assert_eq!(m.path_unit("value.value.sum"), Some(m.unit()));
    }

    /// An invalid declaration is read as one (§2.2): nothing of its
    /// samples is, its unit and kind are unobservable, and its reasons are
    /// the fleet's words.
    #[test]
    fn an_invalid_declaration_reads_nothing_of_its_samples() {
        let l = zenkey_model::contract::load_str(
            "[interface]\nname = \"m\"\nmajor = 1\nuses = [\"telemetry.v1\"]\n\
             [resources.up]\nkind = \"stream\"\ntype = { raw = \"text/plain\" }\n\
             annotations = { \"telemetry.kind\" = \"bool\" }\n",
            std::path::Path::new("."),
            None,
        );
        let rev = zenkey_fleet::Revision::from_contract(
            l.contract.unwrap_or_else(|| panic!("{}", l.report)),
            zenkey_fleet::report::ContractSource::File,
        );
        let mut set = zenkey_fleet::ContractSet::new();
        set.insert(rev);
        let r = resolve(&set, "zk2/lab/m/m.v1/stream/up");
        let m = read(&r, &ring(&[(b"yes", None)])).expect("a metric resource");
        assert!(!m.declared());
        assert_eq!(m.unit(), Unit::Unobservable);
        assert_eq!(m.kind_words(), "unobservable (§2.2)");
        assert!(
            m.invalid().unwrap().contains("kind_value"),
            "{:?}",
            m.invalid()
        );
        assert_eq!(m.samples, 0, "its samples are not read");
        assert!(m.value.is_empty());
    }

    /// A resource that is no metric has no read; its unit is said as such,
    /// and a key no contract resolves says that instead.
    #[test]
    fn a_non_metric_has_no_read_and_says_why_it_has_no_unit() {
        let set = meter();
        let r = resolve(&set, "zk2/lab/meter/meter.v1/stream/frames");
        assert!(read(&r, &ring(&[])).is_some(), "a metric reads empty");
        assert_eq!(unit_of(Some(&r), None), Unit::NotAMetric);
        assert_eq!(unit_of(None, None), Unit::Unresolved);
        assert!(
            Unit::Unknown
                .why_none()
                .unwrap()
                .contains("never read as dimensionless")
        );
        assert_eq!(Unit::Known("By".into(), Source::Declared).why_none(), None);
    }
}

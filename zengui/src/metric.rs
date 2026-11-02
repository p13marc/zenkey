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
//! **The window is the ring's, and it is folded once.** A [`MetricFold`]
//! keeps one book per slot and hands it each ring entry once, in `seq`
//! order, as the ring grows; the charts, the counts and the pairs are the
//! retained entries'. It rebuilds over the retained entries only when it
//! cannot follow the ring (another member, revision or ring generation, a
//! rewind, or entries evicted before they were folded), and then the oldest
//! retained sample closes no pair.

use std::sync::Arc;

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
    /// A narrow counter's change outside its serial window: a wrap or a
    /// decrease, which its values cannot tell apart (telemetry.v2 §2.20).
    pub wrap_or_decrease: u64,
}

/// What the window's history says of one metric member: see the module doc.
#[derive(Debug, Clone)]
pub struct MetricRead {
    /// The resource's declaration (§2.2): `declared`, or `invalid` with its
    /// reasons.
    pub declaration: Declaration,
    /// The member's kind, as its latest sample gave it, else the declared
    /// one; `None` when unobservable (§2.5).
    pub kind: Option<(MetricKind, Source)>,
    /// The member's unit, as its latest sample gave it, else the declared
    /// one; `None` is unknown, never dimensionless (§2.5).
    pub unit: Option<(String, Source)>,
    /// Values read in the window, a delete not counted.
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
    /// Every pair whose later sample the window holds, as the book judged
    /// it (§2.10): a pair stays while its later sample is retained, so the
    /// window's oldest entry may count the pair it closed with a sample
    /// already evicted.
    pub tally: PairTally,
    pub resets: Resets,
    /// The latest decreases within an established epoch, at most
    /// [`DECREASES_KEPT`].
    pub decreases: Vec<Decrease>,
    /// The frame resources of its revision whose series mirror this one,
    /// each `(frame template, field)`: the same quantity carried apart, a
    /// series of its own never paired with this one (telemetry.v2 §2.17).
    pub carried_by: Vec<(String, String)>,
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
        Measure::Counter { value, .. } => Some(*value as f64),
        Measure::Gauge(f) => Some(*f),
        Measure::Boolean(b) => Some(if *b { 1.0 } else { 0.0 }),
        Measure::Histogram(h) => (h.count > 0).then(|| h.sum / h.count as f64),
        Measure::Text(_) | Measure::Binary(_) => None,
    }
}

/// A measure in a few characters, for a finding's subject.
pub fn measure_words(m: &Measure) -> String {
    match m {
        Measure::Counter { value, .. } => value.to_string(),
        Measure::Gauge(f) => f.to_string(),
        Measure::Boolean(b) => b.to_string(),
        Measure::Histogram(h) => format!("count {}", h.count),
        Measure::Text(s) => format!("{s:?}"),
        Measure::Binary(b) => format!("{} bytes", b.len()),
    }
}

/// What a fold reads, apart from its ring's entries: the member and the
/// resource it resolved to, and the ring's generation. Any of it moving is
/// a rebuild.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Inputs {
    /// The member, namespace-relative.
    key: String,
    fingerprint: String,
    index: usize,
    /// [`HistoryRing::generation`]: another ring, or this one cleared.
    generation: u64,
}

impl Inputs {
    fn of(resolved: &Resolved, ring: &HistoryRing) -> Inputs {
        Inputs {
            key: resolved.key.clone(),
            fingerprint: resolved.revision.fingerprint().to_string(),
            index: resolved.index(),
            generation: ring.generation(),
        }
    }
}

/// What the book said of one sample (§2.6), or that it was no sample.
#[derive(Debug, Clone)]
enum Said {
    Delete,
    /// A state sample the late joiner's merge did not keep (§2.10, 0.2).
    NotKept,
    Conforms,
    DoesNot(Vec<Problem>),
    Unobservable(Option<Unasked>),
}

/// One folded ring entry: what the book said of it, its plotted value, and
/// the pair it closed.
#[derive(Debug, Clone)]
struct Folded {
    seq: u64,
    said: Said,
    value: Option<f64>,
    pair: Option<zenkey_fleet::PairSeen>,
}

/// One slot's metric member, folded as its ring grows (review of PR #829):
/// each entry is decoded and judged once, by one [`TelemetryBook`] kept for
/// the fold's life, in `seq` order.
///
/// **A rebuild** — a fresh book over the retained entries — happens only
/// when the fold could not follow the ring: another member, revision or
/// ring generation (a clear, a clone), the ring's newest entry older than
/// the last folded, or entries evicted before they were folded, a hole a
/// pair must not span. The first retained entry of a rebuild closes no
/// pair.
///
/// **Eviction** just drops an entry from the display: the charts, the
/// counts and the pairs are the retained entries'. A pair belongs to its
/// later sample, so it stays while that one is retained, though its earlier
/// sample is gone.
#[derive(Debug, Clone)]
pub struct MetricFold {
    inputs: Inputs,
    metric: Arc<MetricResource>,
    /// The frames whose series mirror this resource (telemetry.v2 §2.17).
    carried_by: Vec<(String, String)>,
    /// Opened at the first entry folded, its origin that entry's arrival:
    /// a fold begun over an empty ring has no arrival to count from yet.
    book: Option<TelemetryBook>,
    /// The last `seq` folded, if any.
    folded: Option<u64>,
    entries: std::collections::VecDeque<Folded>,
    /// Samples handed to the book since the fold began: each entry once.
    decoded: u64,
    read: Arc<MetricRead>,
}

impl MetricFold {
    /// `resolved`'s member, folded over `ring`: `None` when the resource
    /// is no metric resource (§2.1). An invalid declaration is read as one,
    /// and nothing of its samples is (§2.2).
    pub fn new(resolved: &Resolved, ring: &HistoryRing) -> Option<MetricFold> {
        let metric = Arc::new(MetricResource::of(
            &resolved.address,
            &resolved.revision,
            resolved.index(),
        )?);
        // A frame resource's series are [`FrameFold`]'s (telemetry.v2).
        if metric.frame.is_some() {
            return None;
        }
        let carried_by = carried_by(resolved);
        let mut empty = MetricRead::empty(metric.ty.declaration.clone());
        empty.carried_by = carried_by.clone();
        let mut fold = MetricFold {
            inputs: Inputs::of(resolved, ring),
            read: Arc::new(empty),
            carried_by,
            metric,
            book: None,
            folded: None,
            entries: std::collections::VecDeque::new(),
            decoded: 0,
        };
        fold.fold(ring);
        Some(fold)
    }

    /// Whether this fold reads `resolved`'s member through the same
    /// revision: a fold of another member is replaced, not advanced.
    pub fn reads(&self, resolved: &Resolved) -> bool {
        self.inputs.key == resolved.key
            && self.inputs.index == resolved.index()
            && self.inputs.fingerprint == resolved.revision.fingerprint().to_string()
    }

    /// Fold what `ring` appended since the last call, or rebuild when the
    /// fold cannot follow it (the type doc). `true` when the read moved.
    pub fn advance(&mut self, resolved: &Resolved, ring: &HistoryRing) -> bool {
        let oldest = ring.iter().next_back().map(|e| e.seq);
        let newest = ring.newest().map(|e| e.seq);
        let rewound = matches!((self.folded, newest), (Some(f), Some(n)) if n < f)
            || (self.folded.is_some() && newest.is_none());
        let hole = matches!((self.folded, oldest), (Some(f), Some(o)) if o > f + 1);
        if Inputs::of(resolved, ring) != self.inputs || rewound || hole {
            let decoded = self.decoded;
            match MetricFold::new(resolved, ring) {
                Some(mut fresh) => {
                    fresh.decoded += decoded;
                    *self = fresh;
                }
                None => return false,
            }
            return true;
        }
        self.fold(ring)
    }

    /// The read, as of the last fold.
    pub fn read(&self) -> Arc<MetricRead> {
        Arc::clone(&self.read)
    }

    /// Samples decoded since the fold began, rebuilds included: what a tick
    /// costs is what it appended.
    pub fn decoded(&self) -> u64 {
        self.decoded
    }

    /// Fold the entries after the last folded, oldest first, then drop the
    /// ones the ring evicted. `true` when anything moved.
    fn fold(&mut self, ring: &HistoryRing) -> bool {
        let mut moved = false;
        if self.metric.declared() {
            let key = self.inputs.key.clone();
            let after = self.folded;
            let fresh = || ring.iter().filter(|e| after.is_none_or(|f| e.seq > f));
            // Arrivals count from the first entries folded, so every one
            // is at or after the book's origin — never the fold's own
            // instant, which a ring's arrivals may well precede. One
            // arriving before it saturates to it.
            if self.book.is_none()
                && let Some(t0) = fresh().map(|e| e.received).min()
            {
                self.book = Some(TelemetryBook::new(None, t0));
            }
            let newest_first: Vec<&crate::history::HistoryEntry> = fresh().collect();
            for entry in newest_first.into_iter().rev() {
                self.fold_one(&key, entry);
                moved = true;
            }
        }
        if let Some(n) = ring.newest() {
            self.folded = Some(n.seq);
        }
        let oldest = ring.iter().next_back().map(|e| e.seq);
        while self
            .entries
            .front()
            .is_some_and(|e| oldest.is_none_or(|o| e.seq < o))
        {
            self.entries.pop_front();
            moved = true;
        }
        if moved {
            self.read = Arc::new(self.snapshot());
        }
        moved
    }

    fn fold_one(&mut self, key: &str, entry: &crate::history::HistoryEntry) {
        let Some(book) = self.book.as_mut() else {
            return;
        };
        let before = book.member(key).map(counts).unwrap_or_default();
        // The envelope first (core §7.5): the record is what telemetry.v1
        // reads, its run and seq what merge a state member.
        let attachment = entry.attachment.as_ref().map(|a| a.to_bytes().into_owned());
        let (payload, envelope) = zenkey_fleet::model::inband::read_parts(
            key,
            entry.is_delete,
            Some(&entry.encoding),
            entry.payload.to_bytes(),
            attachment.as_deref(),
        );
        let pair = book.sample_enveloped(
            &self.metric,
            MetricSample {
                key: key.to_owned(),
                delete: entry.is_delete,
                payload: payload.into_owned(),
                attachment,
                stamp: entry.stamp,
                at: entry.received,
                via: MetricVia::Subscription,
                frame: None,
            },
            envelope,
        );
        self.decoded += 1;
        let member = book.member(key);
        let after = member.map(counts).unwrap_or_default();
        let said = if after.deletes > before.deletes {
            Said::Delete
        } else if after.conforming > before.conforming {
            Said::Conforms
        } else if after.does_not > before.does_not {
            Said::DoesNot(
                member
                    .and_then(|m| m.last_does_not.clone())
                    .unwrap_or_default(),
            )
        } else if after.unobservable > before.unobservable {
            Said::Unobservable(member.and_then(|m| m.unobservable_why))
        } else {
            Said::NotKept
        };
        let value = match said {
            Said::Conforms => member
                .and_then(|m| m.latest.as_ref())
                .and_then(|r| plotted(&r.value)),
            _ => None,
        };
        self.entries.push_back(Folded {
            seq: entry.seq,
            said,
            value,
            pair,
        });
    }

    /// The read over the retained entries: the charts one point per entry,
    /// the counts and the pairs theirs, the kind and the unit the book's
    /// latest (§2.5).
    fn snapshot(&self) -> MetricRead {
        let mut out = MetricRead::empty(self.metric.ty.declaration.clone());
        out.carried_by = self.carried_by.clone();
        if let Some(m) = self.book.as_ref().and_then(|b| b.member(&self.inputs.key)) {
            if m.kind.is_some() {
                out.kind = m.kind;
            }
            out.unit = m.unit.clone();
        }
        let mut rate = Series::new();
        for e in &self.entries {
            match &e.said {
                Said::Delete => out.deletes += 1,
                Said::NotKept => {}
                Said::Conforms => {
                    out.samples += 1;
                    out.conforming += 1;
                }
                Said::DoesNot(why) => {
                    out.samples += 1;
                    out.does_not += 1;
                    out.last_does_not = Some(why.clone());
                }
                Said::Unobservable(why) => {
                    out.samples += 1;
                    out.unobservable += 1;
                    out.unobservable_why = *why;
                }
            }
            if !matches!(e.said, Said::Delete) {
                out.newest_conforms = matches!(e.said, Said::Conforms);
            }
            match e.value {
                Some(v) => out.value.push(v),
                None => out.value.push_gap(),
            }
            let Some(seen) = &e.pair else {
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
                        NoRate::WrapOrDecrease => out.no_rate.wrap_or_decrease += 1,
                        NoRate::Kind => {}
                    }
                }
            }
            let t = &mut out.tally;
            match seen.pair.continuity {
                Continuity::SameEpoch => t.same_epoch += 1,
                Continuity::Reset => t.reset += 1,
                Continuity::Decreased => t.decreased += 1,
                Continuity::Unobservable => t.unobservable += 1,
                Continuity::NotAsked => t.not_asked += 1,
            }
            match seen.pair.basis {
                Some(Basis::Established(Via::InBand)) => t.in_band += 1,
                Some(Basis::Established(Via::Presence)) => t.by_presence += 1,
                Some(Basis::Assumed) => t.assumed += 1,
                None => {}
            }
            match (seen.pair.continuity, seen.pair.basis) {
                (Continuity::Reset, Some(Basis::Established(Via::InBand))) => {
                    out.resets.in_band += 1;
                }
                (Continuity::Reset, Some(Basis::Established(Via::Presence))) => {
                    out.resets.by_presence += 1;
                }
                (Continuity::Reset, _) => out.resets.assumed += 1,
                // `pair` answers `decreased` only within an established
                // epoch; an assumed decrease is a reset (§2.10).
                (Continuity::Decreased, Some(Basis::Established(via))) => {
                    if out.decreases.len() == DECREASES_KEPT {
                        out.decreases.remove(0);
                    }
                    out.decreases.push(Decrease {
                        seq: e.seq,
                        earlier: seen.earlier.clone(),
                        later: seen.later.clone(),
                        via,
                    });
                }
                _ => {}
            }
        }
        if out.kind.is_some_and(|(k, _)| k == MetricKind::Counter) {
            out.rate = Some(rate);
        }
        out
    }
}

/// A member's running counts, to tell what the book said of the sample just
/// folded.
#[derive(Debug, Clone, Copy, Default)]
struct Counts {
    conforming: u64,
    does_not: u64,
    unobservable: u64,
    deletes: u64,
}

fn counts(m: &zenkey_fleet::MemberBook) -> Counts {
    Counts {
        conforming: m.conforming,
        does_not: m.does_not,
        unobservable: m.unobservable,
        deletes: m.deletes,
    }
}

impl MetricRead {
    /// A read of nothing yet: the declaration's kind and unit (§2.5).
    fn empty(declaration: Declaration) -> MetricRead {
        let declared = declaration.declared().cloned();
        MetricRead {
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
            carried_by: Vec::new(),
        }
    }
}

/// The frame resources of `resolved`'s revision whose declared series
/// mirror its resource, each `(frame template, field)` (telemetry.v2
/// §2.16, §2.17).
fn carried_by(resolved: &Resolved) -> Vec<(String, String)> {
    let c = resolved.revision.contract();
    let template = resolved.resource().template.as_str();
    c.resources
        .iter()
        .filter_map(|r| Some((r, zenkey_fleet::FrameMetric::of(c, r)?)))
        .flat_map(|(r, f)| {
            f.declared()
                .unwrap_or_default()
                .iter()
                .filter(|s| s.mirror.as_deref() == Some(template))
                .map(|s| (r.template.as_str().to_owned(), s.field.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// `resolved`'s metric member over `ring`, read once ([`MetricFold`]):
/// `None` when the resource is no metric resource (§2.1).
pub fn read(resolved: &Resolved, ring: &HistoryRing) -> Option<MetricRead> {
    MetricFold::new(resolved, ring).map(|f| (*f.read()).clone())
}

// ─── telemetry.v2: a frame's series (#983 T2) ──────────────────────────────

/// One series of a frame key, as the window read it (telemetry.v2 §2.17):
/// the frame key and one field, plotted one point per record.
#[derive(Debug, Clone)]
pub struct FieldRead {
    /// Its field, kind, unit, semantic, mirror and width (§2.16).
    pub series: zenkey_model::telemetry::frames::FieldSeries,
    /// One point per record of every retained frame: a sample's plotted
    /// value, else a gap — a record that did not include the field, or held
    /// no value, is never 0 (§2.18).
    pub value: Series,
    /// A counter's rate, one point per record: the rate of the pair the
    /// record closed, else a gap. `None` for any other kind.
    pub rate: Option<Series>,
    pub rate_unit: Option<String>,
    /// The latest rated pair's time basis (§2.21).
    pub basis: Option<zenkey_model::telemetry::TimeBasis>,
    /// Records that included the field with a value (§2.18).
    pub samples: u64,
    /// Records that were no sample, by the reading's token.
    pub no_sample: std::collections::BTreeMap<&'static str, u64>,
    /// The latest sample, core §7.6's JSON reading.
    pub latest: Option<serde_json::Value>,
    pub tally: PairTally,
    pub no_rate: NoRates,
    pub decreases: Vec<Decrease>,
}

impl FieldRead {
    /// Its unit, the entry's or its mirror's; unknown, never
    /// dimensionless, when none gives one (§2.17).
    pub fn unit(&self) -> Unit {
        match &self.series.unit {
            Some(u) => Unit::Known(u.clone(), Source::Declared),
            None => Unit::Unknown,
        }
    }

    /// What its chart plots, as a caption's label; `None` for a text.
    pub fn plotted_label(&self) -> Option<String> {
        let kind = match self.series.kind {
            MetricKind::Counter => match self.series.width {
                Some(w) if w < 64 => format!("counter, modulo 2^{w}"),
                _ => "counter".to_owned(),
            },
            MetricKind::Gauge => "gauge".to_owned(),
            MetricKind::Boolean => "boolean (1 = true, 0 = false)".to_owned(),
            _ => return None,
        };
        Some(format!("{} · {kind}", self.series.field))
    }

    /// Its kind, unit and mirror, for the Inspector's identity.
    pub fn identity_words(&self) -> String {
        let mut w = format!("{} · {}", self.series.kind, self.unit().words());
        if let Some(m) = &self.series.mirror {
            w.push_str(&format!(" · mirrors {m}"));
        }
        w
    }

    /// Its records that were no sample, in words: `""` when none.
    pub fn no_sample_words(&self) -> String {
        self.no_sample
            .iter()
            .map(|(t, n)| format!("{n} {t}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// A frame key's series, as the window read them (telemetry.v2 §2.16–§2.21).
#[derive(Debug, Clone)]
pub struct FrameMetricRead {
    /// Why no field is read: the resource's `telemetry.series` is invalid
    /// (§2.16), in the fleet's words.
    pub invalid: Option<String>,
    /// Each series, in field order.
    pub fields: Vec<FieldRead>,
    /// Frames read, and frames that could not be (a body its layout
    /// refused, a mutation, a refused envelope), each a gap in every chart.
    pub frames: u64,
    pub unread: u64,
}

/// One series of one record, folded.
#[derive(Debug, Clone)]
struct Point {
    sample: bool,
    reading: &'static str,
    value: Option<f64>,
    json: Option<serde_json::Value>,
    pair: Option<zenkey_fleet::PairSeen>,
}

/// One folded ring entry: a frame's records, each its series' points by
/// series index (`None` for a series the fold read nothing of); or a frame
/// that could not be read.
#[derive(Debug, Clone)]
struct FoldedFrame {
    seq: u64,
    unread: bool,
    records: Vec<Vec<Option<Point>>>,
}

/// One slot's frame key, its series folded as its ring grows (telemetry.v2,
/// #983 T2): one [`TelemetryBook`] for the fold's life, whose fold holds
/// the key's frame reader, so a frame lost turns the fields its successors
/// omit `unknown` and the epoch is the run (§2.19). A ring entry is a
/// frame, so the ring's bound counts frames, and a frame of several records
/// several points. It rebuilds, as [`MetricFold`] does, when it cannot
/// follow its ring, its oldest retained frame then its reader's first.
#[derive(Debug, Clone)]
pub struct FrameFold {
    inputs: Inputs,
    metric: Arc<MetricResource>,
    book: Option<TelemetryBook>,
    folded: Option<u64>,
    entries: std::collections::VecDeque<FoldedFrame>,
    decoded: u64,
    read: Arc<FrameMetricRead>,
}

impl FrameFold {
    /// `resolved`'s frame key, folded over `ring`: `None` when its resource
    /// is no frame resource whose `telemetry.series` is declared or
    /// invalid (§2.16).
    pub fn new(resolved: &Resolved, ring: &HistoryRing) -> Option<FrameFold> {
        let metric = Arc::new(MetricResource::of(
            &resolved.address,
            &resolved.revision,
            resolved.index(),
        )?);
        metric.frame.as_ref()?;
        let mut fold = FrameFold {
            inputs: Inputs::of(resolved, ring),
            read: Arc::new(FrameMetricRead {
                invalid: metric.invalid_words(),
                fields: Vec::new(),
                frames: 0,
                unread: 0,
            }),
            metric,
            book: None,
            folded: None,
            entries: std::collections::VecDeque::new(),
            decoded: 0,
        };
        fold.fold(ring);
        fold.read = Arc::new(fold.snapshot());
        Some(fold)
    }

    /// Whether this fold reads `resolved`'s key through the same revision.
    pub fn reads(&self, resolved: &Resolved) -> bool {
        self.inputs.key == resolved.key
            && self.inputs.index == resolved.index()
            && self.inputs.fingerprint == resolved.revision.fingerprint().to_string()
    }

    /// [`MetricFold::advance`]'s, for a frame key.
    pub fn advance(&mut self, resolved: &Resolved, ring: &HistoryRing) -> bool {
        let oldest = ring.iter().next_back().map(|e| e.seq);
        let newest = ring.newest().map(|e| e.seq);
        let rewound = matches!((self.folded, newest), (Some(f), Some(n)) if n < f)
            || (self.folded.is_some() && newest.is_none());
        let hole = matches!((self.folded, oldest), (Some(f), Some(o)) if o > f + 1);
        if Inputs::of(resolved, ring) != self.inputs || rewound || hole {
            let decoded = self.decoded;
            match FrameFold::new(resolved, ring) {
                Some(mut fresh) => {
                    fresh.decoded += decoded;
                    *self = fresh;
                }
                None => return false,
            }
            return true;
        }
        let moved = self.fold(ring);
        if moved {
            self.read = Arc::new(self.snapshot());
        }
        moved
    }

    /// The read, as of the last fold.
    pub fn read(&self) -> Arc<FrameMetricRead> {
        Arc::clone(&self.read)
    }

    /// Frames decoded since the fold began, rebuilds included.
    pub fn decoded(&self) -> u64 {
        self.decoded
    }

    fn fold(&mut self, ring: &HistoryRing) -> bool {
        let mut moved = false;
        if self.metric.declared() {
            let after = self.folded;
            let fresh = || ring.iter().filter(|e| after.is_none_or(|f| e.seq > f));
            if self.book.is_none()
                && let Some(t0) = fresh().map(|e| e.received).min()
            {
                self.book = Some(TelemetryBook::new(None, t0));
            }
            let newest_first: Vec<&crate::history::HistoryEntry> = fresh().collect();
            for entry in newest_first.into_iter().rev() {
                self.fold_one(entry);
                moved = true;
            }
        }
        if let Some(n) = ring.newest() {
            self.folded = Some(n.seq);
        }
        let oldest = ring.iter().next_back().map(|e| e.seq);
        while self
            .entries
            .front()
            .is_some_and(|e| oldest.is_none_or(|o| e.seq < o))
        {
            self.entries.pop_front();
            moved = true;
        }
        moved
    }

    fn fold_one(&mut self, entry: &crate::history::HistoryEntry) {
        use zenkey_fleet::model::inband::Payload;
        let Some(book) = self.book.as_mut() else {
            return;
        };
        let key = self.inputs.key.clone();
        let bytes = entry.payload.to_bytes();
        let encoding = (!entry.encoding.is_empty()).then_some(entry.encoding.as_str());
        let (payload, envelope, frame) =
            match zenkey_fleet::model::inband::payload(&key, encoding, &bytes) {
                Payload::Frame { body, stamp, tag } => (body.to_vec(), Some(Ok(stamp)), Some(tag)),
                Payload::Record { record, stamp, .. } => (record.to_vec(), Some(Ok(stamp)), None),
                Payload::Refused(r) => (bytes.to_vec(), Some(Err(r)), None),
                Payload::Plain(b) | Payload::Heartbeat { body: b, .. } => (b.to_vec(), None, None),
            };
        let seen = book.frame(
            &self.metric,
            MetricSample {
                key,
                delete: entry.is_delete,
                payload,
                attachment: None,
                stamp: entry.stamp,
                at: entry.received,
                via: MetricVia::Subscription,
                frame,
            },
            envelope,
        );
        self.decoded += 1;
        let width = self
            .metric
            .frame
            .as_ref()
            .and_then(|f| f.declared())
            .map_or(0, <[_]>::len);
        let mut records = Vec::new();
        if let Some(folded) = &seen.folded {
            for (r, rec) in folded.records.iter().enumerate() {
                // By series index, as `snapshot` reads them.
                let mut points: Vec<Option<Point>> = vec![None; width];
                for read in &rec.fields {
                    let Some(slot) = points.get_mut(read.series) else {
                        continue;
                    };
                    let sample = read.reading.sample();
                    *slot = Some(Point {
                        sample: sample.is_some(),
                        reading: read.reading.as_str(),
                        value: sample.and_then(|v| plotted(&v.measure)),
                        json: sample.map(|v| v.json.clone()),
                        pair: seen
                            .pairs
                            .iter()
                            .find(|p| p.record == r && p.series == read.series)
                            .map(|p| p.seen.clone()),
                    });
                }
                records.push(points);
            }
        }
        self.entries.push_back(FoldedFrame {
            seq: entry.seq,
            unread: seen.unread.is_some(),
            records,
        });
    }

    /// The read over the retained frames.
    fn snapshot(&self) -> FrameMetricRead {
        let series = self
            .metric
            .frame
            .as_ref()
            .and_then(|f| f.declared())
            .unwrap_or_default();
        let mut out = FrameMetricRead {
            invalid: self.metric.invalid_words(),
            fields: Vec::new(),
            frames: 0,
            unread: 0,
        };
        for e in &self.entries {
            if e.unread {
                out.unread += 1;
            } else if !e.records.is_empty() {
                out.frames += 1;
            }
        }
        for (k, s) in series.iter().enumerate() {
            let counter = s.kind == MetricKind::Counter;
            let mut f = FieldRead {
                series: s.clone(),
                value: Series::new(),
                rate: None,
                rate_unit: None,
                basis: None,
                samples: 0,
                no_sample: std::collections::BTreeMap::new(),
                latest: None,
                tally: PairTally::default(),
                no_rate: NoRates::default(),
                decreases: Vec::new(),
            };
            let mut rate = Series::new();
            for e in &self.entries {
                if e.unread {
                    f.value.push_gap();
                    rate.push_gap();
                    continue;
                }
                for rec in &e.records {
                    // A record keeps its point in every chart: one the fold
                    // read nothing of for this series is a gap.
                    let Some(p) = rec.get(k).and_then(Option::as_ref) else {
                        f.value.push_gap();
                        rate.push_gap();
                        continue;
                    };
                    if p.sample {
                        f.samples += 1;
                        f.latest = p.json.clone();
                    } else {
                        *f.no_sample.entry(p.reading).or_default() += 1;
                    }
                    match p.value {
                        Some(v) => f.value.push(v),
                        None => f.value.push_gap(),
                    }
                    let Some(seen) = &p.pair else {
                        rate.push_gap();
                        continue;
                    };
                    match &seen.pair.rate {
                        Ok(r) => {
                            rate.push(r.per_s);
                            f.rate_unit = r.unit.clone();
                            f.basis = r.basis;
                        }
                        Err(why) => {
                            rate.push_gap();
                            match why {
                                NoRate::Reset => f.no_rate.reset += 1,
                                NoRate::Decreased => f.no_rate.decreased += 1,
                                NoRate::Elapsed => f.no_rate.elapsed += 1,
                                NoRate::WrapOrDecrease => f.no_rate.wrap_or_decrease += 1,
                                NoRate::Kind => {}
                            }
                        }
                    }
                    f.tally.add(&seen.pair);
                    if let (Continuity::Decreased, Some(Basis::Established(via))) =
                        (seen.pair.continuity, seen.pair.basis)
                    {
                        if f.decreases.len() == DECREASES_KEPT {
                            f.decreases.remove(0);
                        }
                        f.decreases.push(Decrease {
                            seq: e.seq,
                            earlier: seen.earlier.clone(),
                            later: seen.later.clone(),
                            via,
                        });
                    }
                }
            }
            if counter {
                f.rate = Some(rate);
            }
            out.fields.push(f);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

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

    /// One bare `By` counter sample, `n`, arriving at `i` half-seconds.
    fn push_count(ring: &mut HistoryRing, t0: Instant, i: u64, n: u64) {
        ring.push(&zenkey_fleet::SampleView {
            key: "k".into(),
            payload: zenoh::bytes::ZBytes::from(n.to_string().into_bytes()),
            encoding: "application/json".into(),
            kind: zenoh::sample::SampleKind::Put,
            timestamp: None,
            stamped_by: None,
            attachment: Some(zenoh::bytes::ZBytes::from(br#""e1""#.to_vec())),
            priority: zenoh::qos::Priority::DEFAULT,
            congestion_control: zenoh::qos::CongestionControl::DEFAULT,
            reliability: zenoh::qos::Reliability::DEFAULT,
            express: false,
            source: None,
            received: t0 + std::time::Duration::from_millis(500 * i),
        });
    }

    /// Review of PR #829: a long run folds each entry once, however many
    /// ticks the window moves on, and eviction only drops entries from the
    /// display. A pair belongs to its later sample, so the window's oldest
    /// entry keeps the rate of the pair it closed with an evicted one.
    #[test]
    fn a_long_run_folds_each_entry_once() {
        let set = meter();
        let r = resolve(&set, "zk2/lab/meter/meter.v1/stream/bytes");
        let t0 = Instant::now();
        let mut ring = HistoryRing::new(16);
        let mut fold = MetricFold::new(&r, &ring).expect("a metric");
        for i in 0..100u64 {
            push_count(&mut ring, t0, i, 50 * i);
            assert!(fold.advance(&r, &ring), "each push moves the read");
            assert!(
                !fold.advance(&r, &ring),
                "a tick with nothing new decodes nothing"
            );
        }
        assert_eq!(fold.decoded(), 100, "each entry decoded once");
        let m = fold.read();
        assert_eq!(m.value.len(), 16, "the window's entries alone");
        assert_eq!(m.samples, 16);
        let rate = m.rate.as_ref().unwrap();
        assert_eq!(rate.points(), [Some(100.0); 16]);
        assert_eq!(m.tally.same_epoch, 16, "a pair stays with its later sample");
        // A one-shot read of the same window agrees, but for the pair the
        // oldest retained entry closed with an evicted one.
        let once = read(&r, &ring).unwrap();
        assert_eq!(once.value.points(), m.value.points());
        assert_eq!(once.rate.as_ref().unwrap().points()[0], None);
    }

    /// A fold begun over an empty ring — a selection, before its first
    /// sample — counts arrivals from its first entry's, never its own
    /// instant: entries that arrived before the fold was made (a scene's
    /// world, a capture's clock) are timed all the same (§2.11).
    #[test]
    fn a_fold_begun_empty_times_what_arrived_before_it() {
        let set = meter();
        let r = resolve(&set, "zk2/lab/meter/meter.v1/stream/bytes");
        let past = Instant::now()
            .checked_sub(std::time::Duration::from_secs(60))
            .unwrap_or_else(Instant::now);
        let mut ring = HistoryRing::new(16);
        let mut fold = MetricFold::new(&r, &ring).unwrap();
        for i in 0..3 {
            push_count(&mut ring, past, i, 50 * i);
        }
        assert!(fold.advance(&r, &ring));
        let m = fold.read();
        assert_eq!(
            m.rate.as_ref().unwrap().points(),
            [None, Some(100.0), Some(100.0)]
        );
        assert_eq!(m.no_rate.elapsed, 0);
    }

    /// Review of PR #829: a fold is never taken for another ring's. A new
    /// ring with the very same `seq` range (a new recorder for the same
    /// member, a pin's clone) is another generation, a clear is too, and
    /// entries evicted before they were folded are a hole no pair spans —
    /// each a rebuild over the retained entries alone.
    #[test]
    fn a_fold_rebuilds_when_it_cannot_follow_its_ring() {
        let set = meter();
        let r = resolve(&set, "zk2/lab/meter/meter.v1/stream/bytes");
        let t0 = Instant::now();
        let mut a = HistoryRing::new(16);
        for i in 0..3 {
            push_count(&mut a, t0, i, 10 * i);
        }
        let mut fold = MetricFold::new(&r, &a).unwrap();
        assert_eq!(
            fold.read().value.points(),
            [Some(0.0), Some(10.0), Some(20.0)]
        );

        // Another ring, the same member, the same seqs 0..=2.
        let mut b = HistoryRing::new(16);
        for i in 0..3 {
            push_count(&mut b, t0, i, 7 * i);
        }
        assert!(fold.advance(&r, &b));
        assert_eq!(
            fold.read().value.points(),
            [Some(0.0), Some(7.0), Some(14.0)],
            "the new ring's, never the old read"
        );
        assert_eq!(fold.decoded(), 6);
        let pinned = b.clone();
        assert_ne!(
            pinned.generation(),
            b.generation(),
            "a clone is another observer"
        );

        // Cleared: another generation, though seqs go on from 3.
        b.clear();
        push_count(&mut b, t0, 3, 5);
        assert!(fold.advance(&r, &b));
        assert_eq!(fold.read().value.points(), [Some(5.0)]);
        assert_eq!(fold.decoded(), 7);

        // A hole: ten pushes into a ring of four between two folds.
        let mut c = HistoryRing::new(4);
        push_count(&mut c, t0, 0, 0);
        let mut fold = MetricFold::new(&r, &c).unwrap();
        for i in 1..11 {
            push_count(&mut c, t0, i, 1000 - i);
        }
        assert!(fold.advance(&r, &c));
        assert_eq!(fold.decoded(), 1 + 4, "the retained four, refolded");
        let m = fold.read();
        assert_eq!(
            m.rate.as_ref().unwrap().points()[0],
            None,
            "no pair across the hole"
        );
        assert_eq!(m.decreases.len(), 3, "the retained pairs' own decreases");
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

    /// `meterframe.v1`, `telemetry.v2`'s scenarios' contract.
    fn meterframe() -> Arc<zenkey_fleet::ContractSet> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../spec/profiles/telemetry/conformance/contracts/meterframe.v1.toml");
        let (set, problems) = zenkey_fleet::ContractSet::load_path(&path);
        assert!(problems.is_empty(), "{problems:?}");
        Arc::new(set)
    }

    /// A ring of `key`'s frames, each in `spec/conformance/frame/bodies.json`'s
    /// spelling, under `layout`, enveloped at run 0001 from seq `seqs[i]`,
    /// arriving a second apart.
    fn frame_ring(r: &Resolved, frames: &[(u32, serde_json::Value)]) -> HistoryRing {
        use zenkey_model::wire::{Run, Seq, Stamp};
        let layout = zenkey_model::frame::Layout::of_resource(r.revision.contract(), r.resource())
            .unwrap()
            .unwrap();
        let t0 = Instant::now();
        let mut ring = HistoryRing::new(64);
        for (i, (seq, v)) in frames.iter().enumerate() {
            let bytes = zenkey_model::frame::Frame::from_json(v, &layout, None)
                .unwrap()
                .envelope(
                    &layout,
                    Stamp {
                        run: Run::U16(1),
                        seq: Seq::new(*seq).unwrap(),
                    },
                )
                .unwrap();
            ring.push(&zenkey_fleet::SampleView {
                key: r.key.clone(),
                payload: zenoh::bytes::ZBytes::from(bytes),
                encoding: "zenoh/bytes".into(),
                kind: zenoh::sample::SampleKind::Put,
                timestamp: None,
                stamped_by: None,
                attachment: None,
                priority: zenoh::qos::Priority::DEFAULT,
                congestion_control: zenoh::qos::CongestionControl::DEFAULT,
                reliability: zenoh::qos::Reliability::DEFAULT,
                express: false,
                source: None,
                received: t0 + std::time::Duration::from_secs(i as u64),
            });
        }
        ring
    }

    /// #983 T2 (telemetry.v2 §2.17–§2.21): a frame key's series, one
    /// point per record. `rx`, a 24-bit counter in frames of three
    /// scheduled records, plots each count and rates each pair under the
    /// schedule; across its wrap the increase is read in its window, and a
    /// step outside it is `wrap_or_decrease`, a gap with no rate.
    #[test]
    fn a_frames_series_plot_one_point_a_record() {
        use serde_json::json;
        let set = meterframe();
        let r = resolve(&set, "zk2/lab/meterframe/meterframe.v1/stream/frames/bytes");
        assert!(MetricFold::new(&r, &HistoryRing::new(4)).is_none(), "v2's");
        let frame = |s: u64, rx: [u64; 3]| {
            json!({"time": {"s": s, "ms": 0}, "records": [
                {"full": true, "values": {"rx": rx[0]}},
                {"full": true, "at": "scheduled", "values": {"rx": rx[1]}},
                {"full": true, "at": "scheduled", "values": {"rx": rx[2]}}]})
        };
        let top = (1u64 << 24) - 1;
        let ring = frame_ring(
            &r,
            &[
                (0, frame(100, [top - 99, top - 98, 100])),
                (3, frame(103, [400, 300, 600])),
            ],
        );
        let fold = FrameFold::new(&r, &ring).expect("a frame resource");
        let read = fold.read();
        assert!(read.invalid.is_none());
        assert_eq!((read.frames, read.unread), (2, 0));
        let rx = &read.fields[0];
        assert_eq!(rx.series.field, "rx");
        assert_eq!(rx.unit(), Unit::Known("By".into(), Source::Declared));
        assert_eq!(rx.value.len(), 6, "one point a record");
        assert_eq!(rx.samples, 6);
        let rate = rx.rate.as_ref().expect("a counter's rate");
        let points = rate.points();
        assert_eq!(points[0], None, "the window's first sample");
        assert_eq!(points[1], Some(1.0), "top - 99 → top - 98 in one second");
        assert_eq!(points[2], Some(199.0), "across the wrap: 199 in its window");
        assert_eq!(points[3], Some(300.0));
        assert_eq!(points[4], None, "400 → 300: outside the window");
        assert_eq!(points[5], Some(300.0));
        assert_eq!(rx.tally.wrap_or_decrease, 1);
        assert_eq!(rx.no_rate.wrap_or_decrease, 1);
        assert!(
            matches!(
                rx.basis,
                Some(zenkey_model::telemetry::TimeBasis::Scheduled { .. })
            ),
            "{:?}",
            rx.basis
        );
        assert_eq!(
            rx.plotted_label().as_deref(),
            Some("rx · counter, modulo 2^24")
        );
    }

    /// A mirrored stream says which frames carry its quantity too, a series
    /// apart (telemetry.v2 §2.17).
    #[test]
    fn a_mirrored_stream_names_the_frames_that_carry_it() {
        let set = meterframe();
        let r = resolve(
            &set,
            "zk2/lab/meterframe/meterframe.v1/stream/sdu/dropped_total",
        );
        let m = read(&r, &ring(&[])).expect("a v1 metric");
        assert_eq!(
            m.carried_by,
            [("frames/link".to_owned(), "dropped".to_owned())]
        );
    }
}

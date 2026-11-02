//! `telemetry.v2`'s frames (`spec/profiles/telemetry/v2.md` §2.16–§2.21,
//! #983 T1): a frame resource's fields bound to metric series.
//!
//! ```text
//! series(fields, annotations, mirror) ─▶ declared [FieldSeries…] │ invalid [problems] │ ignored │ not this profile's
//! reading_of(series, field, slot, state) ─▶ sample │ held │ absent │ owner_error │ beyond_range │ unnamed │ unknown
//! Fold::apply(layout, series, stamp, frame, arrival) ─▶ each record's readings, each counter sample's pair
//! ```
//!
//! - **The declaration** is the frame resource's `telemetry.series`
//!   ([`series`], [`series_of`]): each entry binds one field to a series,
//!   inline (`{kind, unit, semantic}`) or by a **mirror**, a non-frame
//!   metric resource of the interface with no parameter, whose `telemetry.v1`
//!   declaration lends its kind, unit and semantic (v2 §2.16). A field's
//!   packed type carries the kind ([`carries`]).
//! - **The series** is (frame key, field), never the mirrored key (v2
//!   §2.17): the mirror is a label.
//! - **A sample** is a record that includes the field with a value
//!   ([`reading_of`], v2 §2.18). An omitted field holds a value, `known`
//!   by no change or `within` its deadband (core §7.7), and is no sample.
//!   A reserved code, an absent field, the escape and an unknown field are
//!   no value, never 0.
//! - **The epoch is the run** (v2 §2.19): a [`Fold`] reads one key's frames
//!   through a [`crate::frame::Reader`], and two samples are of one epoch
//!   when no frame between them was read as a rewind or another run's.
//! - **A counter** carries its field's width ([`Measure::Counter`]), and
//!   [`crate::telemetry::pair`] reads a narrow one in a serial window (v2
//!   §2.20). Its rate is timed by its records ([`RecordTime`], v2 §2.21).

use std::collections::BTreeMap;
use std::fmt;

use serde_json::Value;

use crate::contract::{Body, Contract, Resource};
use crate::frame::{self, Applied, FieldState, Frame, Layout, Slot, Unknown};
use crate::packed::{Field, FieldKind};
use crate::wire::{Seq, Stamp};

use super::{
    Declaration, Measure, MetricKind, Pair, RecordTime, SERIES, Sampled, Semantic, is_unit, pair,
};

/// The members an inline series may hold (v2 §2.16).
pub const INLINE: &[&str] = &["kind", "unit", "semantic"];

/// One field's series (v2 §2.16, §2.17): which field, and the kind, unit
/// and semantic its entry gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSeries {
    /// The field's name.
    pub field: String,
    /// The field's index in the record type.
    pub index: usize,
    /// `counter`, `gauge`, `boolean` or `text`: what a field carries.
    pub kind: MetricKind,
    pub unit: Option<String>,
    pub semantic: Option<Semantic>,
    /// The mirrored resource's template, for a mirror; `None` inline. A
    /// label, never the series' identity (v2 §2.17).
    pub mirror: Option<String>,
    /// A counter field's width, from 1 to 64; `None` for any other kind.
    pub width: Option<u32>,
}

/// Why a frame resource's `telemetry.series` is invalid (v2 §2.16), in the
/// order a reader checks each entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reason {
    /// The entry names no field of the record type.
    SeriesField,
    /// The annotation is not a non-empty table; or an entry is neither a
    /// string nor a table, or holds a member other than `kind`, `unit` and
    /// `semantic`.
    SeriesValue,
    KindValue,
    UnitValue,
    SemanticValue,
    /// The mirror names no single resource of the interface, or a frame
    /// resource, a template with a parameter, or no metric resource.
    MirrorMissing,
    /// The mirror's own declaration is invalid (v1 §2.2).
    MirrorInvalid,
    /// Nothing gives the series a kind: an inline series with no `kind`,
    /// or a mirror that declares none (a Point with no `telemetry.kind`).
    KindMissing,
    /// The field's type cannot carry the series' kind.
    NotACarrier,
}

impl Reason {
    /// The fixture's token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SeriesValue => "series_value",
            Self::SeriesField => "series_field",
            Self::KindValue => "kind_value",
            Self::UnitValue => "unit_value",
            Self::SemanticValue => "semantic_value",
            Self::KindMissing => "kind_missing",
            Self::MirrorMissing => "mirror_missing",
            Self::MirrorInvalid => "mirror_invalid",
            Self::NotACarrier => "not_a_carrier",
        }
    }

    /// The reason in words, for a linter's or a tool's message.
    #[must_use]
    pub const fn says(self) -> &'static str {
        match self {
            Self::SeriesValue => {
                "telemetry.series is not a table of fields, each a mirror or {kind, unit, semantic}"
            }
            Self::SeriesField => "the entry names no field of the frame's record type",
            Self::KindValue => "kind is not one of the six kinds",
            Self::UnitValue => "unit is not 1 to 64 printable ASCII characters (0x21 to 0x7E)",
            Self::SemanticValue => "semantic is not one of the eight semantics",
            Self::KindMissing => "nothing gives the series a kind",
            Self::MirrorMissing => {
                "the mirror names no single non-frame metric resource of the interface with no parameter"
            }
            Self::MirrorInvalid => "the mirrored resource's own declaration is invalid",
            Self::NotACarrier => "the field's type cannot carry the series' kind",
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One reason, and the entry it is of: `None` for the annotation whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub entry: Option<String>,
    pub reason: Reason,
}

/// What a reader reads of a frame resource (v2 §2.16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Series {
    /// Its series, in field order.
    Declared(Vec<FieldSeries>),
    /// Its problems: the annotation whole first, then each entry in byte
    /// order of its name, each in [`Reason`]'s order.
    Invalid(Vec<Problem>),
    /// No `telemetry.series`, and another `telemetry.*` key: v1's keys mean
    /// nothing on a frame resource.
    Ignored,
    /// No key of the vocabulary.
    NotThisProfiles,
}

impl Series {
    /// The fixture's token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Declared(_) => "declared",
            Self::Invalid(_) => "invalid",
            Self::Ignored => "ignored",
            Self::NotThisProfiles => "not_this_profiles",
        }
    }

    /// The series, when the declaration is valid.
    #[must_use]
    pub fn declared(&self) -> Option<&[FieldSeries]> {
        match self {
            Self::Declared(s) => Some(s),
            _ => None,
        }
    }
}

/// Whether a field's packed type carries `kind` (v2 §2.16): a counter a
/// `counter`, a gauge an `int` or a `fixed`, a boolean a `bool`, a text an
/// `enum`. Nothing carries a histogram or bytes, and a `time` carries no
/// kind.
#[must_use]
pub fn carries(field: &Field, kind: MetricKind) -> bool {
    matches!(
        (kind, &field.kind),
        (MetricKind::Counter, FieldKind::Counter)
            | (
                MetricKind::Gauge,
                FieldKind::Int { .. } | FieldKind::Fixed { .. }
            )
            | (MetricKind::Boolean, FieldKind::Bool)
            | (MetricKind::Text, FieldKind::Enum { .. })
    )
}

/// What an entry gives a series before its field is weighed.
struct Given {
    kind: Option<MetricKind>,
    unit: Option<String>,
    semantic: Option<Semantic>,
    mirror: Option<String>,
}

/// Reads one entry: its reasons, and what it gives when it has none
/// before the carrier's check.
fn entry(value: &Value, mirror: &dyn Fn(&str) -> Option<Declaration>) -> (Vec<Reason>, Given) {
    let mut why = Vec::new();
    let mut given = Given {
        kind: None,
        unit: None,
        semantic: None,
        mirror: None,
    };
    match value {
        Value::Object(m) => {
            if m.keys().any(|k| !INLINE.contains(&k.as_str())) {
                why.push(Reason::SeriesValue);
            }
            let kind = m
                .get("kind")
                .map(|v| v.as_str().and_then(MetricKind::parse));
            let unit = m
                .get("unit")
                .map(|v| v.as_str().filter(|u| is_unit(u)).map(str::to_owned));
            let semantic = m
                .get("semantic")
                .map(|v| v.as_str().and_then(Semantic::parse));
            for (wrong, r) in [
                (matches!(kind, Some(None)), Reason::KindValue),
                (matches!(unit, Some(None)), Reason::UnitValue),
                (matches!(semantic, Some(None)), Reason::SemanticValue),
            ] {
                if wrong {
                    why.push(r);
                }
            }
            if kind.is_none() {
                why.push(Reason::KindMissing);
            }
            given.kind = kind.flatten();
            given.unit = unit.flatten();
            given.semantic = semantic.flatten();
        }
        Value::String(name) => {
            given.mirror = Some(name.clone());
            match mirror(name) {
                None | Some(Declaration::NotThisProfiles | Declaration::Ignored) => {
                    why.push(Reason::MirrorMissing);
                }
                Some(Declaration::Invalid(_)) => why.push(Reason::MirrorInvalid),
                Some(Declaration::Declared(d)) => match d.kind {
                    None => why.push(Reason::KindMissing),
                    Some(k) => {
                        given.kind = Some(k);
                        given.unit = d.unit;
                        given.semantic = d.semantic;
                    }
                },
            }
        }
        _ => why.push(Reason::SeriesValue),
    }
    (why, given)
}

/// Reads a frame resource's series from its record type's `fields` and
/// its resolved annotations (core §9.3, v2 §2.16). `mirror` answers what a
/// mirror names: `None` when it names no single resource of the
/// interface, a frame resource or a template with a parameter, else that
/// resource's `telemetry.v1` declaration ([`super::declaration_of`]).
#[must_use]
pub fn series(
    fields: &[Field],
    annotations: &BTreeMap<String, Value>,
    mirror: &dyn Fn(&str) -> Option<Declaration>,
) -> Series {
    let Some(table) = annotations.get(SERIES) else {
        let v1 = [super::KIND, super::UNIT, super::BUCKETS, super::SEMANTIC];
        return if v1.iter().any(|k| annotations.contains_key(*k)) {
            Series::Ignored
        } else {
            Series::NotThisProfiles
        };
    };
    let whole = |reason| {
        Series::Invalid(vec![Problem {
            entry: None,
            reason,
        }])
    };
    let Some(table) = table.as_object().filter(|t| !t.is_empty()) else {
        return whole(Reason::SeriesValue);
    };
    let mut entries: Vec<(&String, &Value)> = table.iter().collect();
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut problems = Vec::new();
    let mut out = Vec::new();
    for (name, value) in entries {
        let at = fields.iter().position(|f| &f.name == name);
        let (mut why, given) = entry(value, mirror);
        if at.is_none() {
            why.insert(0, Reason::SeriesField);
        }
        if let (Some(i), Some(kind)) = (at, given.kind)
            && !carries(&fields[i], kind)
        {
            why.push(Reason::NotACarrier);
        }
        if why.is_empty() {
            let i = at.expect("an entry with no reason names a field");
            out.push(FieldSeries {
                field: name.clone(),
                index: i,
                kind: given.kind.expect("an entry with no reason has a kind"),
                unit: given.unit,
                semantic: given.semantic,
                mirror: given.mirror,
                width: matches!(fields[i].kind, FieldKind::Counter).then_some(fields[i].width),
            });
        }
        problems.extend(why.into_iter().map(|reason| Problem {
            entry: Some(name.clone()),
            reason,
        }));
    }
    if !problems.is_empty() {
        return Series::Invalid(problems);
    }
    out.sort_by_key(|s| s.index);
    Series::Declared(out)
}

/// What a mirror names in `c` (v2 §2.16): the `telemetry.v1` declaration
/// of the one resource whose template is `name`, when it is no frame
/// resource and has no parameter; `None` otherwise.
#[must_use]
pub fn mirror_in(c: &Contract, name: &str) -> Option<Declaration> {
    let mut named = c.resources.iter().filter(|r| r.template.as_str() == name);
    let r = named.next()?;
    if named.next().is_some() || !r.params.is_empty() {
        return None;
    }
    if let Body::Data(d) = &r.body
        && d.frame.is_some()
    {
        return None;
    }
    Some(super::declaration_of(c, r))
}

/// [`series`] of a contract's frame resource, its mirrors read in the
/// contract ([`mirror_in`]). `None` for a resource that is no frame
/// resource, or whose record type does not resolve (a contract that does
/// not load).
#[must_use]
pub fn series_of(c: &Contract, r: &Resource) -> Option<Series> {
    let layout = Layout::of_resource(c, r).ok().flatten()?;
    Some(series(layout.fields(), &r.annotations, &|name| {
        mirror_in(c, name)
    }))
}

// ---------------------------------------------------------------------------
// Readings (v2 §2.18)
// ---------------------------------------------------------------------------

/// A series field's value: the metric's, and §7.6's JSON reading, which
/// keeps a `fixed` gauge's decimal exact.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldValue {
    pub measure: Measure,
    /// Core §7.6's JSON reading: an integer, a `fixed` field's decimal
    /// string, a boolean or an enum value's name.
    pub json: Value,
}

/// One series field of one record, as a reader reads it (v2 §2.18).
#[derive(Debug, Clone, PartialEq)]
pub enum FieldReading {
    /// Included with a value: a sample.
    Sample(FieldValue),
    /// Not included: the value it holds, `known` by no change since it was
    /// (`within` false) or `within` its deadband. No sample.
    Held { value: FieldValue, within: bool },
    /// No value: included absent, or `not_available`.
    Absent,
    /// No value: `error`, the owner's error.
    OwnerError,
    /// No value: `out_of_range`, beyond the declared range.
    BeyondRange,
    /// No value: a code this revision does not name, an enum's escape.
    Unnamed,
    /// No value: the field's state is `unknown` (core §7.7), with why.
    Unknown(Unknown),
}

impl FieldReading {
    /// The fixture's token.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Sample(_) => "sample",
            Self::Held { .. } => "held",
            Self::Absent => "absent",
            Self::OwnerError => "owner_error",
            Self::BeyondRange => "beyond_range",
            Self::Unnamed => "unnamed",
            Self::Unknown(_) => "unknown",
        }
    }

    /// The sample's value, when it is one.
    #[must_use]
    pub const fn sample(&self) -> Option<&FieldValue> {
        match self {
            Self::Sample(v) => Some(v),
            _ => None,
        }
    }

    /// The value it shows: a sample's, or a held one's. `None` for every
    /// reading that is no value, which a reader never shows as one, nor as
    /// 0.
    #[must_use]
    pub const fn value(&self) -> Option<&FieldValue> {
        match self {
            Self::Sample(v) | Self::Held { value: v, .. } => Some(v),
            _ => None,
        }
    }
}

/// The value of `code` in `field` as a series of `kind`, or `None` for a
/// code it does not name (an enum's escape).
fn value_of(s: &FieldSeries, field: &Field, code: u64) -> Option<FieldValue> {
    let json = field.value(code, None).ok()?;
    let measure = match s.kind {
        MetricKind::Counter => Measure::Counter {
            value: code,
            width: field.width,
        },
        MetricKind::Gauge => Measure::Gauge(match &json {
            Value::String(d) => d.parse::<f64>().ok()?,
            v => v.as_i64()? as f64,
        }),
        MetricKind::Boolean => Measure::Boolean(json.as_bool()?),
        MetricKind::Text => Measure::Text(json.as_str()?.to_owned()),
        MetricKind::Histogram | MetricKind::Binary => return None,
    };
    Some(FieldValue { measure, json })
}

/// Reads one series field of one record (v2 §2.18): its slot in the
/// record (core §7.7's body) and its state as the key's
/// [`crate::frame::Reader`] read it. A sample is a slot that included the
/// field with a code that is a value; an omitted field holding a value is
/// held, no sample; a reserved code, an absent field, an escape and an
/// unknown state are no value, included or held.
#[must_use]
pub fn reading_of(s: &FieldSeries, field: &Field, slot: Slot, state: FieldState) -> FieldReading {
    let (code, within) = match state {
        FieldState::Unknown(why) => return FieldReading::Unknown(why),
        FieldState::Absent => return FieldReading::Absent,
        FieldState::Known(c) => (c, false),
        FieldState::Within(c) => (c, true),
    };
    match field.reserved_name(code) {
        Some("error") => return FieldReading::OwnerError,
        Some("out_of_range") => return FieldReading::BeyondRange,
        Some(_) => return FieldReading::Absent,
        None => {}
    }
    let Some(value) = value_of(s, field, code) else {
        return FieldReading::Unnamed;
    };
    if matches!(slot, Slot::Code(c) if c == code) && !within {
        FieldReading::Sample(value)
    } else {
        FieldReading::Held { value, within }
    }
}

// ---------------------------------------------------------------------------
// A key's fold: readings, epochs and pairs (v2 §2.18–§2.21)
// ---------------------------------------------------------------------------

/// Where a sample lies among a [`Fold`]'s frames: the frame, counted from 0
/// as the fold applied them, and the record's index in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Position {
    pub frame: u64,
    pub record: usize,
}

/// One series field of one record, folded.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesRead {
    /// The series' index in the slice the fold was handed.
    pub series: usize,
    pub reading: FieldReading,
    /// A counter's sample after another of its series: the earlier sample,
    /// and the pair they form.
    pub pair: Option<(Position, Pair)>,
}

/// One record of a frame, folded.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordSeries {
    pub seq: Seq,
    pub time: RecordTime,
    /// One per series, in the series' order.
    pub fields: Vec<SeriesRead>,
}

/// A frame, folded.
#[derive(Debug, Clone, PartialEq)]
pub struct Folded {
    /// The frame as the key's reader read it (core §7.7).
    pub applied: Applied,
    pub records: Vec<RecordSeries>,
}

#[derive(Debug, Clone)]
struct Last {
    measure: Measure,
    epoch: String,
    at: Position,
    time: RecordTime,
    arrival_ns: i128,
}

impl Last {
    fn sampled(&self) -> Sampled<'_> {
        Sampled {
            value: &self.measure,
            epoch: Some(&self.epoch),
            timestamp_ms: None,
            stamp_ns: None,
            arrival_ns: self.arrival_ns,
            record: Some(self.time),
        }
    }
}

/// One frame key's series, as a reader folds its frames (v2 §2.18–§2.21):
/// the key's [`crate::frame::Reader`], the epoch it reads its samples
/// under, and each counter series' last sample.
///
/// - **The epoch is the run.** A frame the reader reads as its first, as
///   another run's or as a rewind starts a new epoch; a gap does not. So
///   two samples are of one epoch, established in band, when no frame
///   between them was read so, and of two otherwise: a pair across a
///   rewind is a reset, never a decrease (v2 §2.19).
/// - **A pair** is two consecutive samples of one counter series, whatever
///   was omitted, unknown or lost between them: two included values are
///   exact (v2 §2.18). It is judged by [`crate::telemetry::pair`], with no
///   presence: the epoch is always in band.
/// - **Each frame** is handed over in the order it arrived, with its
///   arrival on the reader's monotonic clock, the time base of a pair whose
///   record time is unknown (v2 §2.21).
#[derive(Debug, Clone, Default)]
pub struct Fold {
    reader: frame::Reader,
    frames: u64,
    epochs: u64,
    last: BTreeMap<usize, Last>,
}

impl Fold {
    /// A fold that holds nothing: its first frame starts an epoch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The key's reader.
    #[must_use]
    pub const fn reader(&self) -> &frame::Reader {
        &self.reader
    }

    /// Notes a frame at `stamp` its layout refused (core §7.7): never
    /// applied, so the next frame follows a gap ([`frame::Reader::refuse`]).
    pub fn refuse(&mut self, stamp: Stamp) {
        self.reader.refuse(stamp);
    }

    /// Folds a frame of the key: `stamp` its envelope's, `frame` its body
    /// read under `layout`, `series` the resource's declared series
    /// ([`Series::Declared`]) and `arrival_ns` when it arrived, on the
    /// reader's monotonic clock.
    pub fn apply(
        &mut self,
        layout: &Layout,
        series: &[FieldSeries],
        stamp: Stamp,
        frame: &Frame,
        arrival_ns: i128,
    ) -> Folded {
        let applied = self.reader.apply(layout, stamp, frame);
        if applied.follows.resets() && !matches!(applied.follows, frame::Follows::Gap { .. }) {
            self.epochs += 1;
        }
        let epoch = format!("{}/{}", stamp.run.hex(), self.epochs);
        let n = self.frames;
        self.frames += 1;
        let fields = layout.fields();
        let mut records = Vec::with_capacity(frame.records.len());
        for (i, (rec, read)) in frame.records.iter().zip(&applied.records).enumerate() {
            let time = RecordTime {
                frame: n,
                when: read.when,
                after_ms: read.after_ms,
                at_ms: read.at_ms,
            };
            let at = Position {
                frame: n,
                record: i,
            };
            let mut out = Vec::with_capacity(series.len());
            for (k, s) in series.iter().enumerate() {
                let (Some(field), Some(slot), Some(state)) = (
                    fields.get(s.index),
                    rec.slots.get(s.index),
                    read.fields.get(s.index),
                ) else {
                    continue;
                };
                let reading = reading_of(s, field, *slot, *state);
                let pair = match (&reading, s.kind) {
                    (FieldReading::Sample(v), MetricKind::Counter) => {
                        let now = Last {
                            measure: v.measure.clone(),
                            epoch: epoch.clone(),
                            at,
                            time,
                            arrival_ns,
                        };
                        let judged = self.last.get(&s.index).map(|before| {
                            (
                                before.at,
                                pair(&before.sampled(), &now.sampled(), s.unit.as_deref(), None),
                            )
                        });
                        self.last.insert(s.index, now);
                        judged
                    }
                    _ => None,
                };
                out.push(SeriesRead {
                    series: k,
                    reading,
                    pair,
                });
            }
            records.push(RecordSeries {
                seq: read.seq,
                time,
                fields: out,
            });
        }
        Folded { applied, records }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::{Continuity, NoRate, TimeBasis};
    use serde_json::json;

    fn link() -> Vec<Field> {
        let doc = json!({"types": {"Link": {"fields": [
            {"name": "dropped", "type": "counter", "width": 8, "optional": true},
            {"name": "queue_depth", "type": "int", "minimum": 0, "maximum": 1022,
             "reserved": ["error", "out_of_range"], "optional": true},
            {"name": "mode", "type": "enum", "values": ["idle", "tx"], "escape": true},
            {"name": "at", "type": "time"},
        ]}}});
        crate::packed::layout_in(&doc, "Link")
            .unwrap()
            .fields()
            .cloned()
            .collect()
    }

    fn ann(v: Value) -> BTreeMap<String, Value> {
        BTreeMap::from([(SERIES.to_owned(), v)])
    }

    #[test]
    fn an_entry_is_checked_against_its_field_and_its_mirror() {
        let none = |_: &str| None;
        let s = series(
            &link(),
            &ann(json!({"dropped": {"kind": "counter", "unit": "1"}, "mode": {"kind": "text"}})),
            &none,
        );
        let d = s.declared().expect("declared");
        assert_eq!(d[0].width, Some(8));
        assert_eq!(d[1].kind, MetricKind::Text);
        let s = series(
            &link(),
            &ann(json!({"at": {"kind": "gauge"}, "nope": "x/y", "queue_depth": 3})),
            &none,
        );
        let Series::Invalid(p) = s else {
            panic!("{s:?}")
        };
        let got: Vec<_> = p
            .iter()
            .map(|p| (p.entry.as_deref().unwrap(), p.reason.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("at", "not_a_carrier"),
                ("nope", "series_field"),
                ("nope", "mirror_missing"),
                ("queue_depth", "series_value"),
            ]
        );
        assert_eq!(
            series(&link(), &BTreeMap::new(), &none),
            Series::NotThisProfiles
        );
        assert_eq!(series(&link(), &ann(json!({})), &none).as_str(), "invalid");
    }

    #[test]
    fn only_an_included_value_is_a_sample() {
        let f = link();
        let s = FieldSeries {
            field: "queue_depth".into(),
            index: 1,
            kind: MetricKind::Gauge,
            unit: None,
            semantic: None,
            mirror: None,
            width: None,
        };
        let r = |slot, state| reading_of(&s, &f[1], slot, state).as_str();
        assert_eq!(r(Slot::Code(4), FieldState::Known(4)), "sample");
        assert_eq!(r(Slot::Omitted, FieldState::Known(4)), "held");
        assert_eq!(r(Slot::Omitted, FieldState::Within(4)), "held");
        let top = (1u64 << f[1].width) - 1;
        assert_eq!(r(Slot::Code(top), FieldState::Known(top)), "owner_error");
        assert_eq!(
            r(Slot::Code(top - 1), FieldState::Known(top - 1)),
            "beyond_range"
        );
        assert_eq!(r(Slot::Absent, FieldState::Absent), "absent");
    }

    #[test]
    fn a_narrow_counter_reads_in_its_serial_window() {
        let at = |frame, after_ms| RecordTime {
            frame,
            when: frame::When::Measured,
            after_ms,
            at_ms: Some(1_000 * frame + after_ms),
        };
        let s = |v, t| Sampled {
            value: v,
            epoch: Some("r"),
            timestamp_ms: None,
            stamp_ns: None,
            arrival_ns: 0,
            record: Some(t),
        };
        let c = |value| Measure::Counter { value, width: 8 };
        let (a, b, z) = (c(250), c(4), c(0));
        let p = pair(&s(&a, at(0, 0)), &s(&b, at(1, 0)), None, None);
        assert_eq!(p.continuity, Continuity::SameEpoch);
        let r = p.rate.unwrap();
        assert_eq!((r.per_s, r.basis), (10.0, Some(TimeBasis::Measured)));
        let p = pair(&s(&b, at(0, 0)), &s(&z, at(1, 0)), None, None);
        assert_eq!(p.continuity, Continuity::Unobservable);
        assert_eq!(p.rate, Err(NoRate::WrapOrDecrease));
        assert_eq!(p.unobservable(), Some("wrap_or_decrease"));
    }
}

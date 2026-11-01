//! Frames (core §7.7, 0.83; #981's K1): a plain stream's records of one
//! packed type (§7.6) under one envelope (§7.5) and one time.
//!
//! ```text
//! frame, a put's payload   CH · run · seq · body
//! body       time · count · record · … · record · pad
//! time       42 bits: record 0's measured time; all ones when unknown
//! count      the records in the frame, less 1, in ⌈log₂ records⌉ bits
//! record     full · when · offset · fields
//! full       1 bit: 1 for a full record, 0 for a delta record
//! when       1 bit, in record 1 and later only: 1 when the record is scheduled
//! offset     when `when` is 0: the record's time, in ms after record 0's, in ⌈log₂ span⌉ bits
//! fields     a full record: every field, as §7.6 writes it
//!            a delta record: one bit per field, then each field whose bit is 1
//! pad        zero bits up to the next whole byte, fewer than 8
//! ```
//!
//! - **The declaration** is a resource's `frame` table ([`FrameParams`],
//!   from authoring's [`FrameSpec`], E048 its findings), over a packed
//!   record type with no extension marker ([`Layout::of`]).
//! - **The tag** is §7.6's rule over the frame form ([`Layout::form`]): the
//!   table's `records`, `step_ms`, `jitter_ms` and deadbands, and the
//!   record's layout form. `keyframe_every` changes no bit, and is left out.
//! - **A body** is read with its layout ([`Frame::parse`], refusing for the
//!   first of §7.7's checks, [`ReadRefusal`]) and written back to its bytes
//!   ([`Frame::write`]). A record's fields are [`Slot`]s: omitted, included
//!   absent, or a code, which [`Frame::read_json`] spells as §7.6's JSON
//!   reading does.
//!
//! A reader's field states over a key's frames (`known`, `within`,
//! `unknown`, `absent`) are #981's K2, and the owner's writer is K3. This
//! module reads one body, and holds no state across frames.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Duration;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::authoring::{FrameSpec, Kind};
use crate::contract::{Body, Contract, Resource};
use crate::packed::{self, BitReader, BitWriter, Decimal, Field, FieldKind, Item};
use crate::wire::{Envelope, Stamp, Tag};

/// The most records one frame holds.
pub const MAX_RECORDS: u64 = 256;
/// The greatest `step_ms`, 2^32 − 1.
pub const MAX_STEP_MS: u64 = u32::MAX as u64;
/// The greatest `keyframe_every`, and a deadband's greatest `max_records`.
pub const MAX_EVERY: u64 = 65_536;
/// The time code that says the owner's clock is unknown: all 42 bits set.
pub const TIME_UNKNOWN: u64 = (1 << 42) - 1;

/// A field's deadband (core §7.7): the field may be omitted while within
/// `delta` of the value last included, and is included at least once
/// every `max_records` records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deadband {
    /// Above 0, in the field's units. Compared as a value, so `"4.0"` and
    /// `"4"` are one delta.
    pub delta: Decimal,
    /// From 1 to 65,536.
    pub max_records: u32,
}

/// A frame resource's `frame` table, resolved (core §7.7): every key has
/// its value, a default's included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameParams {
    /// From 1 to 256.
    pub records: u16,
    /// From 1 to 2^32 − 1, exactly when `records` is above 1.
    pub step_ms: Option<u32>,
    /// From 0 to `step_ms` / 2; set exactly when `step_ms` is.
    pub jitter_ms: Option<u32>,
    /// From 1 to 65,536.
    pub keyframe_every: u32,
    /// By field name.
    pub deadband: BTreeMap<String, Deadband>,
}

impl FrameParams {
    /// The table as written, resolved; or E048's findings, one sentence
    /// each, for every key and deadband member outside §7.7's table.
    ///
    /// # Errors
    /// Each value out of range, in words.
    pub fn resolve(spec: &FrameSpec) -> Result<Self, Vec<String>> {
        let mut why = Vec::new();
        let records = spec.records.unwrap_or(1);
        if !(1..=MAX_RECORDS).contains(&records) {
            why.push(format!("`records` = {records} is not from 1 to 256"));
        }
        match spec.step_ms {
            Some(s) if !(1..=MAX_STEP_MS).contains(&s) => {
                why.push(format!("`step_ms` = {s} is not from 1 to 2^32 - 1"));
            }
            Some(_) if records == 1 => {
                why.push("`step_ms` with `records` = 1: a one-record frame has no step".to_owned())
            }
            None if records > 1 => why.push(format!(
                "`records` = {records} and no `step_ms`: a frame of several records has a step"
            )),
            _ => {}
        }
        match (spec.jitter_ms, spec.step_ms) {
            (Some(_), None) => why.push("`jitter_ms` without `step_ms`".to_owned()),
            (Some(j), Some(s)) if j.saturating_mul(2) > s => {
                why.push(format!("`jitter_ms` = {j} is above `step_ms` / 2"));
            }
            _ => {}
        }
        let keyframe_every = spec.keyframe_every.unwrap_or(1);
        if !(1..=MAX_EVERY).contains(&keyframe_every) {
            why.push(format!(
                "`keyframe_every` = {keyframe_every} is not from 1 to 65,536"
            ));
        }
        let mut deadband = BTreeMap::new();
        for (name, d) in &spec.deadband {
            // Kept in its shortest form, so `"4.0"` and `"4"` are one value.
            let delta = Decimal::parse(&d.delta)
                .filter(|x| *x > Decimal::from(0))
                .and_then(|x| Decimal::parse(&x.to_string()));
            if delta.is_none() {
                why.push(format!(
                    "`deadband.{name}.delta` = {:?} is not a decimal string above 0",
                    d.delta
                ));
            }
            if !(1..=MAX_EVERY).contains(&d.max_records) {
                why.push(format!(
                    "`deadband.{name}.max_records` = {} is not from 1 to 65,536",
                    d.max_records
                ));
            }
            if let (Some(delta), Ok(max_records)) = (delta, u32::try_from(d.max_records)) {
                deadband.insert(name.clone(), Deadband { delta, max_records });
            }
        }
        if !why.is_empty() {
            return Err(why);
        }
        let step_ms = spec.step_ms.map(|s| u32::try_from(s).expect("checked"));
        Ok(Self {
            records: u16::try_from(records).expect("checked"),
            step_ms,
            jitter_ms: step_ms.map(|_| {
                u32::try_from(spec.jitter_ms.unwrap_or(0)).expect("checked below the step")
            }),
            keyframe_every: u32::try_from(keyframe_every).expect("checked"),
            deadband,
        })
    }

    /// The span, `records` × `step_ms`, where the frame has a step.
    #[must_use]
    pub fn span_ms(&self) -> Option<u64> {
        self.step_ms.map(|s| u64::from(self.records) * u64::from(s))
    }

    /// The frame resource's interval (core §7.7): its span where it has a
    /// step, else the `timing.period_ms` its annotations declare as
    /// `timing.v1` §2.2 reads them, else unstated.
    #[must_use]
    pub fn interval_ms(&self, annotations: &BTreeMap<String, Value>) -> Option<u64> {
        self.span_ms().or_else(
            || match crate::timing::declaration(Kind::Stream, annotations) {
                crate::timing::Declaration::Declared(d) => d.period.map(millis),
                _ => None,
            },
        )
    }

    /// Its canonical form's member (core §9.5): `records`, `step_ms` and
    /// `jitter_ms` (`null` without a step), `keyframe_every`, and
    /// `deadband` by field name, each `delta` in its shortest form.
    #[must_use]
    pub fn canonical(&self) -> Value {
        let deadband: Map<String, Value> = self
            .deadband
            .iter()
            .map(|(k, d)| {
                (
                    k.clone(),
                    json!({"delta": d.delta.to_string(), "max_records": d.max_records}),
                )
            })
            .collect();
        json!({
            "records": self.records,
            "step_ms": self.step_ms,
            "jitter_ms": self.jitter_ms,
            "keyframe_every": self.keyframe_every,
            "deadband": deadband,
        })
    }

    /// Its canonical form's member read back, as a bundle carries it.
    ///
    /// # Errors
    /// What is wrong with it, in words.
    pub fn from_canonical(v: &Value) -> Result<Self, String> {
        let m = v.as_object().ok_or("`frame` is not an object")?;
        let int = |k: &str| -> Result<Option<u64>, String> {
            match m.get(k) {
                None => Err(format!("`frame` has no `{k}`")),
                Some(Value::Null) => Ok(None),
                Some(x) => x
                    .as_u64()
                    .map(Some)
                    .ok_or_else(|| format!("`frame.{k}` is not an unsigned integer")),
            }
        };
        let mut deadband = BTreeMap::new();
        let db = m
            .get("deadband")
            .and_then(Value::as_object)
            .ok_or("`frame.deadband` is not an object")?;
        for (name, d) in db {
            let delta = d
                .get("delta")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("`frame.deadband.{name}` has no `delta` string"))?;
            let max_records = d
                .get("max_records")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("`frame.deadband.{name}` has no `max_records`"))?;
            deadband.insert(
                name.clone(),
                crate::authoring::DeadbandSpec {
                    delta: delta.to_owned(),
                    max_records,
                },
            );
        }
        let step_ms = int("step_ms")?;
        let jitter_ms = int("jitter_ms")?;
        if step_ms.is_some() != jitter_ms.is_some() {
            return Err("`frame.jitter_ms` is null exactly when `step_ms` is".to_owned());
        }
        let spec = FrameSpec {
            records: Some(int("records")?.ok_or("`frame.records` is null")?),
            step_ms,
            jitter_ms,
            keyframe_every: Some(int("keyframe_every")?.ok_or("`frame.keyframe_every` is null")?),
            deadband,
        };
        Self::resolve(&spec).map_err(|why| why.join("; "))
    }
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// ⌈log₂ n⌉, the fewest bits that hold n codes: 0 for one.
fn bits_for(n: u64) -> u32 {
    if n <= 1 {
        0
    } else {
        64 - (n - 1).leading_zeros()
    }
}

/// A frame resource's layout (core §7.7): its record type's fields, its
/// table, and the frame form its tag comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub params: FrameParams,
    fields: Vec<Field>,
    /// Each field's deadband, in field order.
    deadband: Vec<Option<Deadband>>,
    form: Value,
    sha256: [u8; 32],
}

impl Layout {
    /// The layout of a frame of `record`'s type under `params`.
    ///
    /// # Errors
    /// Why there is none: the record type has an extension marker (E047),
    /// or a deadband names no field of it, or one that is not an `int`,
    /// `fixed` or `counter` (E049).
    pub fn of(record: &packed::Layout, params: &FrameParams) -> Result<Self, String> {
        let mut fields = Vec::new();
        for it in &record.items {
            match it {
                Item::Marker => {
                    return Err(
                        "the record type has an extension marker, which a frame's type never has"
                            .to_owned(),
                    );
                }
                Item::Field(f) => fields.push(f.clone()),
            }
        }
        for name in params.deadband.keys() {
            match fields.iter().find(|f| &f.name == name) {
                None => return Err(format!("the deadband names {name:?}, no field of the type")),
                Some(f) if !takes_deadband(f) => {
                    return Err(format!(
                        "the deadband names {name:?}, a {} field, which takes none",
                        f.kind.as_str()
                    ));
                }
                Some(_) => {}
            }
        }
        let deadband: Vec<Option<Deadband>> = fields
            .iter()
            .map(|f| params.deadband.get(&f.name).cloned())
            .collect();
        let items: Vec<Value> = deadband
            .iter()
            .map(|d| match d {
                None => Value::Null,
                Some(d) => json!({"delta": d.delta.to_string(), "max_records": d.max_records}),
            })
            .collect();
        let form = json!({
            "frame": {
                "records": params.records,
                "step_ms": params.step_ms,
                "jitter_ms": params.jitter_ms,
                "deadband": items,
            },
            "record": record.form().clone(),
        });
        let sha256 = Sha256::digest(crate::schema::jcs(&form)).into();
        Ok(Self {
            params: params.clone(),
            fields,
            deadband,
            form,
            sha256,
        })
    }

    /// The layout of a contract's frame resource: `None` for a resource
    /// that is no frame resource.
    ///
    /// # Errors
    /// Why a frame resource has none: its type's layout (an artifact
    /// missing, or a declaration with a finding), or [`Layout::of`]'s.
    pub fn of_resource(c: &Contract, r: &Resource) -> Result<Option<Self>, String> {
        let Body::Data(d) = &r.body else {
            return Ok(None);
        };
        let Some(params) = &d.frame else {
            return Ok(None);
        };
        let record = c
            .packed_layout(&d.type_)?
            .ok_or_else(|| format!("{}: a frame resource's type is not packed", r.template))?;
        Self::of(&record, params).map(Some)
    }

    /// The frame form: the root of the tag (core §7.7).
    #[must_use]
    pub fn form(&self) -> &Value {
        &self.form
    }

    /// The sha256 of the frame form's JCS bytes, in lowercase hex.
    #[must_use]
    pub fn sha256_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.sha256 {
            let _ = write!(s, "{b:02x}");
        }
        s
    }

    /// The tag CH carries: the low four bits of the hash's last byte.
    #[must_use]
    pub fn tag(&self) -> Tag {
        Tag::new(self.sha256[31] & 0x0f).expect("four bits")
    }

    /// The count's width, ⌈log₂ records⌉: none for a one-record frame.
    #[must_use]
    pub fn count_bits(&self) -> u32 {
        bits_for(u64::from(self.params.records))
    }

    /// The offset's width, ⌈log₂ span⌉, where the frame has a step.
    #[must_use]
    pub fn offset_bits(&self) -> Option<u32> {
        self.params.span_ms().map(bits_for)
    }

    /// The span, where the frame has a step.
    #[must_use]
    pub fn span_ms(&self) -> Option<u64> {
        self.params.span_ms()
    }

    /// The record type's fields, in order.
    #[must_use]
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// The deadband of the field at `index`, if it has one.
    #[must_use]
    pub fn deadband(&self, index: usize) -> Option<&Deadband> {
        self.deadband.get(index).and_then(Option::as_ref)
    }

    /// A record's time as a span of milliseconds after record 0: a
    /// scheduled record lies within `jitter_ms` of its instant.
    fn bounds(&self, i: usize, at: Option<At>) -> (u64, u64) {
        match at {
            None => (0, 0),
            Some(At::Offset(o)) => (o, o),
            Some(At::Scheduled) => {
                let step = u64::from(self.params.step_ms.unwrap_or(0));
                let jitter = u64::from(self.params.jitter_ms.unwrap_or(0));
                let t = step * i as u64;
                (t.saturating_sub(jitter), t + jitter)
            }
        }
    }
}

/// Whether a field takes a deadband: an `int`, `fixed` or `counter`.
#[must_use]
pub fn takes_deadband(f: &Field) -> bool {
    matches!(
        f.kind,
        FieldKind::Int { .. } | FieldKind::Fixed { .. } | FieldKind::Counter
    )
}

/// Record 0's time: seconds from the deployment epoch and milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Time {
    pub s: u32,
    /// From 0 to 999.
    pub ms: u16,
}

/// A later record's time, after record 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum At {
    /// `step_ms` × i after record 0, within `jitter_ms`: an instant and a
    /// bound, never a measurement.
    Scheduled,
    /// Measured: this many milliseconds after record 0.
    Offset(u64),
}

/// One field of a record, as the body carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    /// A delta record's field whose bit is 0.
    Omitted,
    /// Included, with its presence bit 0.
    Absent,
    /// Included, with this code.
    Code(u64),
}

/// One record of a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// A full record includes every field.
    pub full: bool,
    /// `None` for record 0, whose time is the frame's.
    pub at: Option<At>,
    /// One per field, in field order.
    pub slots: Vec<Slot>,
}

/// A frame's body, read: record 0's time, `None` when the owner's clock
/// was unknown, and its records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub time: Option<Time>,
    pub records: Vec<Record>,
}

/// Why a reader refused a body (core §7.7's order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadRefusal {
    /// CH's tag is not the frame's.
    Tag,
    /// The body ends before a bit its fields need, or holds a byte after
    /// its pad.
    Length,
    /// A code is unused: a millisecond code from 1000 up but the unknown
    /// time's, a count of `records` or more, an offset at or past the
    /// span, or a field's unused code.
    Code,
    /// A record lies before the record before it.
    Order,
    /// A bit of the pad is 1.
    Pad,
}

impl ReadRefusal {
    /// The token `spec/conformance/frame/bodies.json` pins.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Tag => "tag",
            Self::Length => "length",
            Self::Code => "code",
            Self::Order => "order",
            Self::Pad => "pad",
        }
    }

    /// The refusal in words, its token first.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Tag => "tag: the envelope's tag is not the frame's",
            Self::Length => "length: the body is not as long as its fields read it",
            Self::Code => "code: a code is unused",
            Self::Order => "order: a record lies before the record before it",
            Self::Pad => "pad: a bit of the pad is 1",
        }
    }
}

impl std::fmt::Display for ReadRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason())
    }
}

impl std::error::Error for ReadRefusal {}

impl From<packed::ReadRefusal> for ReadRefusal {
    fn from(r: packed::ReadRefusal) -> Self {
        match r {
            packed::ReadRefusal::Pad => Self::Pad,
            packed::ReadRefusal::Length => Self::Length,
            packed::ReadRefusal::Tag => Self::Tag,
            _ => Self::Code,
        }
    }
}

/// Why a writer refused a frame (core §7.7, the fixtures' tokens).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteRefusal {
    /// No record, or more than `records`.
    Records,
    /// A full record without a field that is not optional, or such a field
    /// written absent.
    Missing,
    /// A field the type does not have.
    Unknown,
    /// A value of the wrong JSON type, a name not declared, or a record
    /// whose time is not its place's (record 0 with one, a later one
    /// without).
    Value,
    /// A value with no code, an offset at or past the span, or a time
    /// outside its codes.
    Range,
    /// A record that lies before the record before it.
    Order,
}

impl WriteRefusal {
    /// The token `spec/conformance/frame/bodies.json` pins.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Records => "records",
            Self::Missing => "missing",
            Self::Unknown => "unknown",
            Self::Value => "value",
            Self::Range => "range",
            Self::Order => "order",
        }
    }
}

impl std::fmt::Display for WriteRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.token())
    }
}

impl std::error::Error for WriteRefusal {}

impl From<packed::WriteRefusal> for WriteRefusal {
    fn from(r: packed::WriteRefusal) -> Self {
        match r {
            packed::WriteRefusal::Missing => Self::Missing,
            packed::WriteRefusal::Unknown => Self::Unknown,
            packed::WriteRefusal::Value => Self::Value,
            packed::WriteRefusal::Range => Self::Range,
        }
    }
}

impl Frame {
    /// A body read under the tag its envelope's CH carries, refusing it for
    /// the first of core §7.7's checks that applies.
    ///
    /// # Errors
    /// The reader's refusal: [`ReadRefusal`].
    pub fn parse(tag: Tag, body: &[u8], layout: &Layout) -> Result<Frame, ReadRefusal> {
        if tag != layout.tag() {
            return Err(ReadRefusal::Tag);
        }
        let mut r = BitReader::new(body);
        let t = r.take(42)?;
        let time = if t == TIME_UNKNOWN {
            None
        } else {
            let ms = (t & 0x3ff) as u16;
            if ms >= 1000 {
                return Err(ReadRefusal::Code);
            }
            Some(Time {
                s: u32::try_from(t >> 10).expect("32 bits"),
                ms,
            })
        };
        let count = r.take(layout.count_bits())?;
        if count >= u64::from(layout.params.records) {
            return Err(ReadRefusal::Code);
        }
        let n = usize::try_from(count).expect("below 256") + 1;
        let mut records = Vec::with_capacity(n);
        let mut earliest = 0;
        for i in 0..n {
            let full = r.bit()?;
            let at = if i == 0 {
                None
            } else if r.bit()? {
                Some(At::Scheduled)
            } else {
                let o = r.take(layout.offset_bits().unwrap_or(0))?;
                if layout.span_ms().is_none_or(|span| o >= span) {
                    return Err(ReadRefusal::Code);
                }
                Some(At::Offset(o))
            };
            let (lo, hi) = layout.bounds(i, at);
            if hi < earliest {
                return Err(ReadRefusal::Order);
            }
            earliest = lo;
            let included: Vec<bool> = if full {
                vec![true; layout.fields.len()]
            } else {
                (0..layout.fields.len())
                    .map(|_| r.bit())
                    .collect::<Result<_, _>>()?
            };
            let mut slots = Vec::with_capacity(layout.fields.len());
            for (f, inc) in layout.fields.iter().zip(included) {
                if !inc {
                    slots.push(Slot::Omitted);
                } else if f.optional && !r.bit()? {
                    slots.push(Slot::Absent);
                } else {
                    let code = r.take(f.width)?;
                    f.check_code(code)?;
                    slots.push(Slot::Code(code));
                }
            }
            records.push(Record { full, at, slots });
        }
        r.end()?;
        Ok(Frame { time, records })
    }

    /// The body's bits, then the pad: what [`Frame::parse`] reads back.
    ///
    /// # Errors
    /// The writer's refusal: [`WriteRefusal`].
    pub fn write(&self, layout: &Layout) -> Result<packed::Record, WriteRefusal> {
        let n = self.records.len();
        if n == 0 || n > usize::from(layout.params.records) {
            return Err(WriteRefusal::Records);
        }
        let mut w = BitWriter::new();
        match self.time {
            None => w.push(TIME_UNKNOWN, 42),
            Some(t) if t.ms >= 1000 => return Err(WriteRefusal::Range),
            Some(t) => w.push((u64::from(t.s) << 10) | u64::from(t.ms), 42),
        }
        w.push(n as u64 - 1, layout.count_bits());
        let mut earliest = 0;
        for (i, rec) in self.records.iter().enumerate() {
            if rec.slots.len() != layout.fields.len() {
                return Err(WriteRefusal::Value);
            }
            w.bit(rec.full);
            match (i, rec.at) {
                (0, None) => {}
                (0, Some(_)) | (_, None) => return Err(WriteRefusal::Value),
                (_, Some(At::Scheduled)) => w.bit(true),
                (_, Some(At::Offset(o))) => {
                    if layout.span_ms().is_none_or(|span| o >= span) {
                        return Err(WriteRefusal::Range);
                    }
                    w.bit(false);
                    w.push(o, layout.offset_bits().unwrap_or(0));
                }
            }
            let (lo, hi) = layout.bounds(i, rec.at);
            if hi < earliest {
                return Err(WriteRefusal::Order);
            }
            earliest = lo;
            if !rec.full {
                for s in &rec.slots {
                    w.bit(!matches!(s, Slot::Omitted));
                }
            }
            for (f, s) in layout.fields.iter().zip(&rec.slots) {
                match s {
                    Slot::Omitted if rec.full => return Err(WriteRefusal::Missing),
                    Slot::Omitted => {}
                    Slot::Absent if !f.optional => return Err(WriteRefusal::Missing),
                    Slot::Absent => w.bit(false),
                    Slot::Code(c) => {
                        if (f.width < 64 && *c >> f.width != 0) || f.check_code(*c).is_err() {
                            return Err(WriteRefusal::Range);
                        }
                        if f.optional {
                            w.bit(true);
                        }
                        w.push(*c, f.width);
                    }
                }
            }
        }
        Ok(w.finish())
    }

    /// The put's payload: the frame's envelope, its tag the layout's, its
    /// stamp record 0's, then the body.
    ///
    /// # Errors
    /// [`Frame::write`]'s.
    pub fn envelope(&self, layout: &Layout, stamp: Stamp) -> Result<Vec<u8>, WriteRefusal> {
        let body = self.write(layout)?;
        Ok(Envelope::frame(layout.tag(), stamp, &body.bytes).write())
    }

    /// The frame as `spec/conformance/frame/bodies.json` spells a value:
    /// `time`, `{"s", "ms"}` or `null`, and `records`, each `full`, `at`
    /// (after record 0: `"scheduled"` or its offset), `values` (the fields
    /// included present, as §7.6's JSON reading spells them) and `absent`
    /// (those included absent, in field order).
    ///
    /// # Errors
    /// [`ReadRefusal::Code`] where a `time` field's instant lies past an
    /// `i64` of milliseconds under `epoch` ([`packed::ReadRefusal::Epoch`]),
    /// which no epoch within ±9.2 × 10^15 seconds gives.
    pub fn read_json(&self, layout: &Layout, epoch: Option<i64>) -> Result<Value, ReadRefusal> {
        let mut records = Vec::new();
        for rec in &self.records {
            let mut m = Map::new();
            m.insert("full".into(), rec.full.into());
            match rec.at {
                None => {}
                Some(At::Scheduled) => {
                    m.insert("at".into(), "scheduled".into());
                }
                Some(At::Offset(o)) => {
                    m.insert("at".into(), o.into());
                }
            }
            let mut values = Map::new();
            let mut absent = Vec::new();
            for (f, s) in layout.fields.iter().zip(&rec.slots) {
                match s {
                    Slot::Omitted => {}
                    Slot::Absent => absent.push(Value::from(f.name.clone())),
                    Slot::Code(c) => {
                        values.insert(f.name.clone(), f.value(*c, epoch)?);
                    }
                }
            }
            m.insert("values".into(), Value::Object(values));
            m.insert("absent".into(), Value::Array(absent));
            records.push(Value::Object(m));
        }
        let time = self
            .time
            .map_or(Value::Null, |t| json!({"s": t.s, "ms": t.ms}));
        Ok(json!({"time": time, "records": records}))
    }

    /// A frame from [`Frame::read_json`]'s spelling, each field's value
    /// coded as §7.6's writer codes it. A full record's optional field in
    /// neither `values` nor `absent` is absent.
    ///
    /// # Errors
    /// The writer's refusal: [`WriteRefusal`].
    pub fn from_json(
        v: &Value,
        layout: &Layout,
        epoch: Option<i64>,
    ) -> Result<Frame, WriteRefusal> {
        let time = match v.get("time") {
            Some(Value::Null) => None,
            Some(t) => {
                let s = t
                    .get("s")
                    .and_then(Value::as_u64)
                    .ok_or(WriteRefusal::Value)?;
                let ms = t
                    .get("ms")
                    .and_then(Value::as_u64)
                    .ok_or(WriteRefusal::Value)?;
                Some(Time {
                    s: u32::try_from(s).map_err(|_| WriteRefusal::Range)?,
                    ms: u16::try_from(ms)
                        .ok()
                        .filter(|ms| *ms < 1000)
                        .ok_or(WriteRefusal::Range)?,
                })
            }
            None => return Err(WriteRefusal::Value),
        };
        let recs = v
            .get("records")
            .and_then(Value::as_array)
            .ok_or(WriteRefusal::Value)?;
        if recs.is_empty() || recs.len() > usize::from(layout.params.records) {
            return Err(WriteRefusal::Records);
        }
        let mut records = Vec::with_capacity(recs.len());
        for r in recs {
            let full = r
                .get("full")
                .and_then(Value::as_bool)
                .ok_or(WriteRefusal::Value)?;
            let at = match r.get("at") {
                None => None,
                Some(Value::String(s)) if s == "scheduled" => Some(At::Scheduled),
                Some(x) => Some(At::Offset(x.as_u64().ok_or(WriteRefusal::Value)?)),
            };
            let empty = Map::new();
            let values = match r.get("values") {
                None => &empty,
                Some(x) => x.as_object().ok_or(WriteRefusal::Value)?,
            };
            let absent: Vec<&str> = match r.get("absent") {
                None => Vec::new(),
                Some(x) => x
                    .as_array()
                    .ok_or(WriteRefusal::Value)?
                    .iter()
                    .map(|n| n.as_str().ok_or(WriteRefusal::Value))
                    .collect::<Result<_, _>>()?,
            };
            let known = |n: &str| layout.fields.iter().any(|f| f.name == n);
            if values.keys().any(|k| !known(k)) || absent.iter().any(|k| !known(k)) {
                return Err(WriteRefusal::Unknown);
            }
            let mut slots = Vec::with_capacity(layout.fields.len());
            for f in &layout.fields {
                let slot = match values.get(&f.name) {
                    // A counter's count is written modulo 2^width (§7.6).
                    Some(x) => Slot::Code(f.code(x, epoch)? & packed::top_code(f.width)),
                    None if absent.contains(&f.name.as_str()) || (full && f.optional) => {
                        if !f.optional {
                            return Err(WriteRefusal::Missing);
                        }
                        Slot::Absent
                    }
                    None if full => return Err(WriteRefusal::Missing),
                    None => Slot::Omitted,
                };
                slots.push(slot);
            }
            records.push(Record { full, at, slots });
        }
        Ok(Frame { time, records })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link() -> packed::Layout {
        let doc = json!({"types": {"Link": {"fields": [
            {"name": "dropped", "type": "counter", "width": 32, "optional": true},
            {"name": "queue_depth", "type": "int", "minimum": 0, "maximum": 1022,
             "reserved": ["out_of_range"], "optional": true},
            {"name": "duty_available", "type": "int", "minimum": 0, "maximum": 1, "optional": true},
            {"name": "duty_wait_ms", "type": "fixed", "minimum": "0", "maximum": "3600000",
             "resolution": "100", "reserved": ["out_of_range"], "optional": true}
        ]}}});
        packed::layout_in(&doc, "Link").expect("a layout")
    }

    fn one() -> FrameParams {
        FrameParams::resolve(&FrameSpec::default()).expect("the defaults")
    }

    /// zenoh-modem's four link readings, one record a frame (#981's §3):
    /// a 63-bit record, so a keyframe's body is 14 bytes and its payload 20.
    #[test]
    fn zenoh_modems_link_frame() {
        let l = Layout::of(&link(), &one()).expect("a frame layout");
        assert_eq!(
            (l.count_bits(), l.offset_bits(), l.span_ms()),
            (0, None, None)
        );
        let all = json!({"time": {"s": 3600, "ms": 250}, "records": [{"full": true,
            "values": {"dropped": 7, "queue_depth": 12, "duty_available": 1, "duty_wait_ms": "1500"},
            "absent": []}]});
        let f = Frame::from_json(&all, &l, None).expect("a frame");
        let body = f.write(&l).expect("written");
        assert_eq!((body.bits, body.bytes.len()), (106, 14));
        let stamp = Stamp {
            run: crate::wire::Run::U16(1),
            seq: crate::wire::Seq::new(12).unwrap(),
        };
        let put = f.envelope(&l, stamp).expect("an envelope");
        assert_eq!(put.len(), 20);
        let e = Envelope::parse(&put).expect("parsed");
        assert!(e.is_frame());
        let back = Frame::parse(e.tag, e.record, &l).expect("read back");
        assert_eq!(back, f);
        assert_eq!(back.read_json(&l, None).unwrap(), all);
        // A delta record with the queue and the wait changed: 10 bytes.
        let d = json!({"time": {"s": 3660, "ms": 0}, "records": [{"full": false,
            "values": {"queue_depth": 30, "duty_wait_ms": "2500"}, "absent": []}]});
        let body = Frame::from_json(&d, &l, None).unwrap().write(&l).unwrap();
        assert_eq!((body.bits, body.bytes.len()), (75, 10));
        // Nothing changed: 6 bytes.
        let d = json!({"time": {"s": 3720, "ms": 0}, "records": [{"full": false}]});
        let body = Frame::from_json(&d, &l, None).unwrap().write(&l).unwrap();
        assert_eq!((body.bits, body.bytes.len()), (47, 6));
    }

    #[test]
    fn the_table_resolves_or_names_each_finding() {
        let p = one();
        assert_eq!(
            (p.records, p.step_ms, p.jitter_ms, p.keyframe_every),
            (1, None, None, 1)
        );
        let p = FrameParams::resolve(&FrameSpec {
            records: Some(3),
            step_ms: Some(10_000),
            ..FrameSpec::default()
        })
        .unwrap();
        assert_eq!((p.jitter_ms, p.span_ms()), (Some(0), Some(30_000)));
        let bad = FrameParams::resolve(&FrameSpec {
            records: Some(0),
            jitter_ms: Some(1),
            keyframe_every: Some(0),
            ..FrameSpec::default()
        })
        .unwrap_err();
        assert_eq!(bad.len(), 3, "{bad:?}");
        assert_eq!(FrameParams::from_canonical(&p.canonical()), Ok(p));
    }

    #[test]
    fn widths() {
        assert_eq!(
            (
                bits_for(1),
                bits_for(2),
                bits_for(3),
                bits_for(256),
                bits_for(257)
            ),
            (0, 1, 2, 8, 9)
        );
        assert_eq!(bits_for(256 * MAX_STEP_MS), 40);
    }
}

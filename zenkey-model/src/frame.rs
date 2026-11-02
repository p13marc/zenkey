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
//! - **A key's frames** are read by a [`Reader`] (#981's K2): what each
//!   frame follows in the run it holds ([`Follows`]), and each record's
//!   seq, time ([`When`]) and field states ([`FieldState`]: `known`,
//!   `within`, `unknown` with why, `absent`), as §7.7's "The reader's
//!   fields" says; [`Applied::reading`] spells them as
//!   `spec/conformance/frame/apply.json` does, which observer §7.6's tools
//!   show.
//!
//! - **A key's writer** is a [`Framer`] (#981's K3): the records an owner
//!   measured ([`Measured`]) gathered into frames ([`Batch`]), each written
//!   under its run ([`Framer::encode`]) as §7.7's "The owner's frames"
//!   says, full where the run is new or `keyframe_every` asks, each delta
//!   field included where it changed or passed its deadband.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Duration;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::authoring::{FrameSpec, Kind};
use crate::contract::{Body, Contract, Resource};
use crate::packed::{self, BitReader, BitWriter, Decimal, Field, FieldKind, Item};
use crate::wire::{Envelope, Run, Seq, Stamp, Tag};

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

/// What a frame follows, in the run a reader holds (core §7.7, "The
/// frames it expects").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Follows {
    /// The reader held no run: its first frame of the key.
    First,
    /// The seq after the last record it read, in the run it holds.
    Continues,
    /// A newer seq by core §4.3's serial order: `lost` records lost.
    Gap { lost: u32 },
    /// Any other seq of the run it holds, one exactly 2^23 ahead included:
    /// the key restarted under a run it had used, which §4.3 forbids.
    Rewind,
    /// A run other than the one it holds.
    Run,
}

impl Follows {
    /// `spec/conformance/frame/apply.json`'s token.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Continues => "continues",
            Self::Gap { .. } => "gap",
            Self::Rewind => "rewind",
            Self::Run => "run",
        }
    }

    /// Whether the reader turned every field `unknown` at this frame.
    #[must_use]
    pub const fn resets(self) -> bool {
        !matches!(self, Self::Continues)
    }
}

/// Why a field is `unknown` (core §7.7's table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unknown {
    /// Not included since the reader's first frame of the key.
    First,
    /// Not included since a frame of a run the reader did not hold.
    Run,
    /// Not included since records were lost.
    Gap,
    /// Not included since a rewind.
    Rewind,
    /// `max_records` records after the record that included it.
    Deadband,
    /// Omitted under a deadband after an inclusion that was no value: the
    /// field absent, or a reserved code.
    NoValue,
}

impl Unknown {
    /// `spec/conformance/frame/apply.json`'s token.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Run => "run",
            Self::Gap => "gap",
            Self::Rewind => "rewind",
            Self::Deadband => "deadband",
            Self::NoValue => "no_value",
        }
    }

    /// Why, in words, for a person (observer §7.6).
    #[must_use]
    pub const fn words(self) -> &'static str {
        match self {
            Self::First => "not included since the first frame read",
            Self::Run => "not included since a run not held",
            Self::Gap => "not included since records were lost",
            Self::Rewind => "not included since a rewind",
            Self::Deadband => "max_records passed since it was included",
            Self::NoValue => "omitted under a deadband after no value",
        }
    }

    fn of(follows: Follows) -> Self {
        match follows {
            Follows::First => Self::First,
            Follows::Run => Self::Run,
            Follows::Gap { .. } => Self::Gap,
            Follows::Rewind | Follows::Continues => Self::Rewind,
        }
    }
}

/// A field's state in one record (core §7.7, "The reader's fields").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldState {
    /// Its value, at the record's time: included in this record, or
    /// omitted with no deadband and nothing lost since it was included.
    Known(u64),
    /// The last value included, give or take its deadband's `delta`:
    /// omitted under a deadband, fewer than `max_records` records after a
    /// record that included a value.
    Within(u64),
    /// Nothing, with why: never its last value, never 0.
    Unknown(Unknown),
    /// Nothing: the owner wrote it absent.
    Absent,
}

impl FieldState {
    /// `spec/conformance/frame/apply.json`'s token.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Known(_) => "known",
            Self::Within(_) => "within",
            Self::Unknown(_) => "unknown",
            Self::Absent => "absent",
        }
    }

    /// The code it holds: a `known` field's value, or a `within` field's
    /// last value (give or take its delta). `None` for `unknown` and
    /// `absent`, which hold none.
    #[must_use]
    pub const fn code(self) -> Option<u64> {
        match self {
            Self::Known(c) | Self::Within(c) => Some(c),
            Self::Unknown(_) | Self::Absent => None,
        }
    }
}

/// A record's time, as a reader reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum When {
    /// Measured: record 0's on the owner's wall clock, a later record's as
    /// its offset on the owner's monotonic clock from record 0.
    Measured,
    /// `step_ms` × its index after record 0, within `jitter_ms`: an
    /// instant and a bound, never a measurement (observer §7.6).
    Scheduled { jitter_ms: u32 },
    /// Record 0 of a frame whose time is unknown: the owner's clock was.
    Unknown,
}

impl When {
    /// `spec/conformance/frame/apply.json`'s token.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Measured => "measured",
            Self::Scheduled { .. } => "scheduled",
            Self::Unknown => "unknown",
        }
    }
}

/// One record of a frame, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordRead {
    /// Record 0's seq plus its index, modulo 2^24.
    pub seq: Seq,
    pub when: When,
    /// After record 0: its offset, or `step_ms` × its index when
    /// scheduled. 0 for record 0.
    pub after_ms: u64,
    /// Its instant in milliseconds from the deployment epoch: record 0's
    /// time plus `after_ms`, `None` when that time is unknown.
    pub at_ms: Option<u64>,
    /// One per field, in field order.
    pub fields: Vec<FieldState>,
}

/// A frame as a [`Reader`] read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// The envelope's: record 0's seq.
    pub stamp: Stamp,
    pub follows: Follows,
    pub records: Vec<RecordRead>,
    /// Record 0's seq of each frame of the run it holds that its layout
    /// refused since the frame before this one ([`Reader::refuse`]), in
    /// arrival order. Their records are among a gap's `lost`: a refused
    /// frame's body is not read, so how many records it held is not known
    /// to the reader, and only the next frame's seq tells it.
    pub refused: Vec<Seq>,
}

impl Applied {
    /// The frame as `spec/conformance/frame/apply.json` spells a reading:
    /// `run`, `seq`, `follows` (and `lost` for a gap), then each record's
    /// `seq`, `time`, `after_ms` and `jitter_ms` where they apply, `at`
    /// (`{s, ms}`) where its instant is known, and `fields` by name, each
    /// `{state}` with its `value` (§7.6's JSON reading), a `within`
    /// field's `delta`, and an `unknown` field's `why`.
    ///
    /// # Errors
    /// [`Frame::read_json`]'s: a `time` field's instant past an `i64` of
    /// milliseconds under `epoch`.
    pub fn reading(&self, layout: &Layout, epoch: Option<i64>) -> Result<Value, ReadRefusal> {
        let mut records = Vec::with_capacity(self.records.len());
        for (index, rec) in self.records.iter().enumerate() {
            let mut m = Map::new();
            m.insert("seq".into(), rec.seq.get().into());
            m.insert("time".into(), rec.when.token().into());
            if index > 0 {
                m.insert("after_ms".into(), rec.after_ms.into());
            }
            if let When::Scheduled { jitter_ms } = rec.when {
                m.insert("jitter_ms".into(), jitter_ms.into());
            }
            if let Some(at) = rec.at_ms {
                m.insert("at".into(), json!({"s": at / 1000, "ms": at % 1000}));
            }
            let mut fields = Map::new();
            for (i, (f, s)) in layout.fields.iter().zip(&rec.fields).enumerate() {
                let mut v = Map::new();
                v.insert("state".into(), s.token().into());
                match s {
                    FieldState::Known(c) => {
                        v.insert("value".into(), f.value(*c, epoch)?);
                    }
                    FieldState::Within(c) => {
                        v.insert("value".into(), f.value(*c, epoch)?);
                        if let Some(d) = layout.deadband(i) {
                            v.insert("delta".into(), d.delta.to_string().into());
                        }
                    }
                    FieldState::Unknown(why) => {
                        v.insert("why".into(), why.token().into());
                    }
                    FieldState::Absent => {}
                }
                fields.insert(f.name.clone(), Value::Object(v));
            }
            m.insert("fields".into(), Value::Object(fields));
            records.push(Value::Object(m));
        }
        let mut out = Map::new();
        out.insert("run".into(), self.stamp.run.hex().into());
        out.insert("seq".into(), self.stamp.seq.get().into());
        out.insert("follows".into(), self.follows.token().into());
        if let Follows::Gap { lost } = self.follows {
            out.insert("lost".into(), lost.into());
        }
        out.insert("records".into(), Value::Array(records));
        Ok(Value::Object(out))
    }
}

/// What a field's last inclusion was, since the reader last reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Last {
    /// Not included since the reset.
    Never,
    Absent,
    /// A value's code.
    Value(u64),
    /// A reserved code: present, and no value.
    Reserved(u64),
}

#[derive(Debug, Clone, Copy)]
struct Track {
    last: Last,
    /// Records since its last inclusion.
    since: u32,
}

#[derive(Debug, Clone)]
struct Held {
    run: Run,
    /// The seq after the last record read.
    next: Seq,
    /// Why every field turned unknown at the last reset.
    reset: Unknown,
    /// The layout's hash: another layout is read as another run.
    layout: [u8; 32],
    tracks: Vec<Track>,
    /// Frames of this run its layout refused since the last one read.
    refused: Vec<Seq>,
}

/// One key's reader (core §7.7, "The reader's fields"): what it holds of
/// the key's frames, in the run it holds. A consumer or a tool keeps one
/// per frame key and hands it each frame in the order it arrived
/// ([`Reader::apply`]). A frame refused by its layout is never applied: its
/// body is not read, so neither its fields nor how many records it held
/// are known, and the next frame's seq reads its records as lost, a gap
/// (core §7.7, "The frames it expects": a seq newer than the one expected
/// follows records lost, whatever lost them; `spec/conformance/frame/
/// apply.json` reads a gap so). [`Reader::refuse`] notes it, so the next
/// frame says which of its lost records were refused ([`Applied::refused`]).
#[derive(Debug, Clone, Default)]
pub struct Reader {
    held: Option<Held>,
}

impl Reader {
    /// A reader that holds nothing: its first frame is [`Follows::First`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The run it holds, and the seq it expects next.
    #[must_use]
    pub fn expects(&self) -> Option<Stamp> {
        self.held.as_ref().map(|h| Stamp {
            run: h.run,
            seq: h.next,
        })
    }

    /// Notes a frame at `stamp` whose layout refused it (core §7.7's
    /// refusals). It is never applied: the reader still expects the seq
    /// after the last record it read, so the next frame follows a gap
    /// counting the refused frame's records, and lists its seq in
    /// [`Applied::refused`]. A refused frame of a run it does not hold is
    /// forgotten: the next frame of that run resets every field anyway.
    pub fn refuse(&mut self, stamp: Stamp) {
        if let Some(h) = &mut self.held
            && h.run == stamp.run
            && h.refused.len() < usize::from(u16::MAX)
        {
            h.refused.push(stamp.seq);
        }
    }

    /// What a frame at `stamp` would follow, read under `layout`.
    #[must_use]
    pub fn follows(&self, layout: &Layout, stamp: Stamp) -> Follows {
        let Some(h) = &self.held else {
            return Follows::First;
        };
        if h.run != stamp.run || h.layout != layout.sha256 {
            return Follows::Run;
        }
        const SPACE: u32 = 1 << 24;
        const HALF: u32 = 1 << 23;
        let d = stamp.seq.get().wrapping_sub(h.next.get()) & (SPACE - 1);
        match d {
            0 => Follows::Continues,
            d if d < HALF => Follows::Gap { lost: d },
            _ => Follows::Rewind,
        }
    }

    /// Reads a frame of the key, `stamp` its envelope's, `frame` its body
    /// read under `layout` ([`Frame::parse`]): every field turns
    /// `unknown` at a gap, a rewind and a run not held (its first frame
    /// included), then each record is read in turn, a field `known` or
    /// `absent` where included, carried where omitted with no deadband,
    /// `within` under its deadband until `max_records` records after it
    /// was included with a value, and `unknown` past it.
    ///
    /// A reader holds one layout: a frame under another (another revision
    /// of the resource) is read as one of a run it does not hold.
    pub fn apply(&mut self, layout: &Layout, stamp: Stamp, frame: &Frame) -> Applied {
        let follows = self.follows(layout, stamp);
        let n = layout.fields.len();
        // A frame refused in the run held is among what this one follows.
        let refused = self
            .held
            .as_mut()
            .filter(|h| h.run == stamp.run)
            .map(|h| std::mem::take(&mut h.refused))
            .unwrap_or_default();
        let held = match (&mut self.held, follows.resets()) {
            (Some(h), false) => h,
            (slot, _) => slot.insert(Held {
                run: stamp.run,
                next: stamp.seq,
                reset: Unknown::of(follows),
                layout: layout.sha256,
                tracks: vec![
                    Track {
                        last: Last::Never,
                        since: 0,
                    };
                    n
                ],
                refused: Vec::new(),
            }),
        };
        let base = frame.time.map(|t| u64::from(t.s) * 1000 + u64::from(t.ms));
        let mut records = Vec::with_capacity(frame.records.len());
        for (i, rec) in frame.records.iter().enumerate() {
            let seq = Seq::new((stamp.seq.get() + i as u32) & Seq::MAX.get()).expect("24 bits");
            let (when, after_ms) = match rec.at {
                None if base.is_none() => (When::Unknown, 0),
                None => (When::Measured, 0),
                Some(At::Offset(o)) => (When::Measured, o),
                Some(At::Scheduled) => (
                    When::Scheduled {
                        jitter_ms: layout.params.jitter_ms.unwrap_or(0),
                    },
                    u64::from(layout.params.step_ms.unwrap_or(0)) * i as u64,
                ),
            };
            let mut fields = Vec::with_capacity(n);
            for (k, (f, slot)) in layout.fields.iter().zip(&rec.slots).enumerate() {
                let t = &mut held.tracks[k];
                let state = match slot {
                    Slot::Code(c) => {
                        *t = Track {
                            last: if f.reserved_name(*c).is_some() {
                                Last::Reserved(*c)
                            } else {
                                Last::Value(*c)
                            },
                            since: 0,
                        };
                        FieldState::Known(*c)
                    }
                    Slot::Absent => {
                        *t = Track {
                            last: Last::Absent,
                            since: 0,
                        };
                        FieldState::Absent
                    }
                    Slot::Omitted => {
                        t.since = t.since.saturating_add(1);
                        match (t.last, layout.deadband(k)) {
                            (Last::Never, _) => FieldState::Unknown(held.reset),
                            (Last::Absent, None) => FieldState::Absent,
                            (Last::Value(c) | Last::Reserved(c), None) => FieldState::Known(c),
                            (Last::Absent | Last::Reserved(_), Some(_)) => {
                                FieldState::Unknown(Unknown::NoValue)
                            }
                            (Last::Value(_), Some(d)) if t.since >= d.max_records => {
                                FieldState::Unknown(Unknown::Deadband)
                            }
                            (Last::Value(c), Some(_)) => FieldState::Within(c),
                        }
                    }
                };
                fields.push(state);
            }
            records.push(RecordRead {
                seq,
                when,
                after_ms,
                at_ms: base.map(|b| b + after_ms),
                fields,
            });
        }
        let count = u32::try_from(frame.records.len()).expect("at most 256");
        held.next = Seq::new((stamp.seq.get() + count) & Seq::MAX.get()).expect("24 bits");
        Applied {
            stamp,
            follows,
            records,
            refused,
        }
    }
}

// ── The owner's frames (core §7.7, "The owner's frames"; #981's K3) ─────

impl Time {
    /// The time of an instant `t`, in milliseconds since the Unix epoch,
    /// under the deployment epoch `epoch` in seconds: `None` for one before
    /// the epoch or 2^32 seconds after it, which the owner writes as
    /// unknown.
    #[must_use]
    pub fn from_unix_ms(t: i128, epoch: i64) -> Option<Time> {
        let delta = t - 1000 * i128::from(epoch);
        let (s, ms) = (delta.div_euclid(1000), delta.rem_euclid(1000));
        Some(Time {
            s: u32::try_from(s).ok()?,
            ms: u16::try_from(ms).expect("below 1000"),
        })
    }

    /// The instant in milliseconds since the Unix epoch, under `epoch`.
    #[must_use]
    pub fn unix_ms(self, epoch: i64) -> i128 {
        1000 * (i128::from(epoch) + i128::from(self.s)) + i128::from(self.ms)
    }
}

impl Layout {
    /// A §7.6 record of the frame's record type read into a full record's
    /// slots: each field absent or its code. The record's own tag is not
    /// asked: its bytes are the type's.
    ///
    /// # Errors
    /// [`ReadRefusal::Length`], [`ReadRefusal::Code`] or
    /// [`ReadRefusal::Pad`], as §7.6's reader refuses the record.
    pub fn slots_of(&self, record: &[u8]) -> Result<Vec<Slot>, ReadRefusal> {
        let mut r = BitReader::new(record);
        let mut slots = Vec::with_capacity(self.fields.len());
        for f in &self.fields {
            if f.optional && !r.bit()? {
                slots.push(Slot::Absent);
                continue;
            }
            let code = r.take(f.width)?;
            f.check_code(code)?;
            slots.push(Slot::Code(code));
        }
        r.end()?;
        Ok(slots)
    }

    /// A record in §7.6's JSON reading, a field's name to its value, read
    /// into a full record's slots: an optional field left out or `null` is
    /// absent. A `time` field counts from `epoch`.
    ///
    /// # Errors
    /// [`WriteRefusal`]: `value` for a value that is no object, `unknown`
    /// for a name the type does not have, `missing` for a field that is not
    /// optional left out, and §7.6's refusals of a value.
    pub fn slots_of_json(&self, v: &Value, epoch: Option<i64>) -> Result<Vec<Slot>, WriteRefusal> {
        let m = v.as_object().ok_or(WriteRefusal::Value)?;
        if m.keys().any(|k| !self.fields.iter().any(|f| &f.name == k)) {
            return Err(WriteRefusal::Unknown);
        }
        self.fields
            .iter()
            .map(|f| match m.get(&f.name) {
                None | Some(Value::Null) if f.optional => Ok(Slot::Absent),
                None => Err(WriteRefusal::Missing),
                Some(x) => Ok(Slot::Code(f.code(x, epoch)? & packed::top_code(f.width))),
            })
            .collect()
    }

    /// The bits of the largest frame an owner writes: `records` full
    /// records, every optional field present, and every record after
    /// record 0 measured, with its offset (link.v1 §2.3's
    /// `frame_over_batch`).
    #[must_use]
    pub fn largest_body_bits(&self) -> u64 {
        let record: u64 = 1 + self
            .fields
            .iter()
            .map(|f| u64::from(f.width) + u64::from(f.optional))
            .sum::<u64>();
        let n = u64::from(self.params.records);
        let later = n.saturating_sub(1) * (1 + u64::from(self.offset_bits().unwrap_or(0)));
        42 + u64::from(self.count_bits()) + n * record + later
    }

    /// The payload of that largest frame, in bytes: its body after an
    /// envelope of a 32-bit run, 8 bytes (core §7.5).
    #[must_use]
    pub fn largest_payload_bytes(&self) -> u64 {
        8 + self.largest_body_bits().div_ceil(8)
    }

    /// The value of a field's code a deadband weighs: an `int`'s integer,
    /// a `fixed`'s decimal, a `counter`'s count. `None` for a reserved code,
    /// which holds no value, and for a field that takes no deadband.
    fn weighed(&self, index: usize, code: u64) -> Option<Decimal> {
        let f = &self.fields[index];
        if f.reserved_name(code).is_some() {
            return None;
        }
        match &f.kind {
            FieldKind::Int {
                minimum, maximum, ..
            } => packed::int_value(code, *minimum, *maximum)
                .ok()
                .map(Decimal::of_i128),
            FieldKind::Fixed {
                minimum,
                resolution,
                values,
                ..
            } => packed::fixed_value(code, minimum, resolution, *values).ok(),
            FieldKind::Counter => Some(Decimal::of_i128(i128::from(code))),
            _ => None,
        }
    }
}

/// One record an owner measured: each field absent or its code
/// ([`Slot::Absent`], [`Slot::Code`]), when it measured it on its
/// monotonic clock (`at`), and its wall clock's time then, `None` while
/// its clock is unknown. Only record 0's wall time is written: a later
/// record's time is its offset on the monotonic clock (core §7.7,
/// "Times").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measured {
    pub slots: Vec<Slot>,
    pub at: crate::timing::Mono,
    pub wall: Option<Time>,
}

/// The records of one frame, in the order they were measured: what a
/// [`Framer`] hands its owner to write ([`Framer::encode`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch(Vec<Measured>);

impl Batch {
    /// How many records it holds, and so how many seqs its frame takes
    /// (core §4.3).
    #[must_use]
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> u32 {
        u32::try_from(self.0.len()).expect("at most 256")
    }

    /// Its records.
    #[must_use]
    pub fn records(&self) -> &[Measured] {
        &self.0
    }
}

/// What the key's readers hold after the last frame an owner sent, in the
/// run it was sent under: what the next delta record is written against.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sent {
    run: Run,
    /// The last record sent, every field absent or a code.
    prev: Vec<Slot>,
    /// Each field as the last record that included it held it.
    included: Vec<Slot>,
    /// Records since each field was last included.
    since: Vec<u32>,
    /// Records since the last full record.
    since_full: u32,
}

/// A batch written as a frame under a run ([`Framer::encode`]), and what
/// the key's readers hold once it is sent: [`Framer::sent`] keeps that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    pub frame: Frame,
    next: Sent,
}

/// One frame key's writer (core §7.7, "The owner's frames"): a pure state
/// machine an owner hands each record it measures, in the order it
/// measured them ([`Framer::push`]), which says when a frame is complete
/// and writes it ([`Framer::encode`]). The owner takes the frame's seqs,
/// one per record (§4.3), puts it, and tells the framer it was sent
/// ([`Framer::sent`]). A frame never sent leaves nothing behind: the
/// next is written against the last one sent.
///
/// - **Frames.** A frame holds up to `records` records. A record that
///   would lie at or past the open frame's span ends it, and opens the
///   next as its record 0. [`Framer::flush`] ends the open frame early.
/// - **Full records.** Record 0 of the first frame written under a run is
///   full, and so is a record `keyframe_every` records after the last full
///   one. A delta record that would include every field is written full.
/// - **Delta records** include each field with no deadband whose code or
///   presence differs from the record before. A field with a deadband is
///   omitted while it holds a value within `delta` of the value last
///   included, as that inclusion did, and fewer than `max_records`
///   records passed since; else it is included.
/// - **Times.** Record 0 carries its wall time. A later record is
///   scheduled when its offset from record 0, on the monotonic clock, lies
///   within `jitter_ms` of `step_ms` × its index, and carries its offset,
///   in whole milliseconds, otherwise.
#[derive(Debug, Clone)]
pub struct Framer {
    layout: Layout,
    open: Vec<Measured>,
    last_at: Option<crate::timing::Mono>,
    sent: Option<Sent>,
}

impl Framer {
    /// A framer of `layout`'s frames that has sent none.
    #[must_use]
    pub fn new(layout: Layout) -> Self {
        Self {
            layout,
            open: Vec::new(),
            last_at: None,
            sent: None,
        }
    }

    /// Its layout.
    #[must_use]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// How many records the open frame holds.
    #[must_use]
    pub fn open(&self) -> usize {
        self.open.len()
    }

    /// The run the last frame sent was written under.
    #[must_use]
    pub fn run(&self) -> Option<Run> {
        self.sent.as_ref().map(|s| s.run)
    }

    /// Takes a measured record, refused before anything is kept: a slot
    /// omitted, one per field missing, a field that is not optional
    /// written absent, a code its field does not use, or a record measured
    /// before the last one.
    ///
    /// Returns the frame now complete, if one is: the open frame, when
    /// this record lies at or past its span (this record then opens the
    /// next frame), or the open frame with this record, when that holds
    /// `records` records. A one-record frame is complete at once.
    ///
    /// # Errors
    /// [`WriteRefusal`]: `value` for a slot omitted or the wrong count,
    /// `missing`, `range`, `order`.
    pub fn push(&mut self, m: Measured) -> Result<Option<Batch>, WriteRefusal> {
        if m.slots.len() != self.layout.fields.len() {
            return Err(WriteRefusal::Value);
        }
        for (f, s) in self.layout.fields.iter().zip(&m.slots) {
            match s {
                Slot::Omitted => return Err(WriteRefusal::Value),
                Slot::Absent if !f.optional => return Err(WriteRefusal::Missing),
                Slot::Absent => {}
                Slot::Code(c) => {
                    if (f.width < 64 && *c >> f.width != 0) || f.check_code(*c).is_err() {
                        return Err(WriteRefusal::Range);
                    }
                }
            }
        }
        if self.last_at.is_some_and(|l| m.at < l) {
            return Err(WriteRefusal::Order);
        }
        self.last_at = Some(m.at);
        let past_span = match (self.open.first(), self.layout.params.span_ms()) {
            (Some(first), Some(span)) => {
                m.at.since_origin().saturating_sub(first.at.since_origin())
                    >= Duration::from_millis(span)
            }
            _ => false,
        };
        let ended = if past_span {
            Some(Batch(std::mem::take(&mut self.open)))
        } else {
            None
        };
        self.open.push(m);
        if ended.is_some() {
            return Ok(ended);
        }
        if self.open.len() >= usize::from(self.layout.params.records) {
            return Ok(Some(Batch(std::mem::take(&mut self.open))));
        }
        Ok(None)
    }

    /// Ends the open frame now, if it holds a record: an owner that puts
    /// what it measured so far, or stops.
    pub fn flush(&mut self) -> Option<Batch> {
        (!self.open.is_empty()).then(|| Batch(std::mem::take(&mut self.open)))
    }

    /// `batch` written as a frame under `run`: record 0 full when the last
    /// frame sent was another run's, or none was; each later record's time
    /// and fields as [`Framer`] says.
    #[must_use]
    pub fn encode(&self, batch: &Batch, run: Run) -> Encoded {
        let l = &self.layout;
        let n = l.fields.len();
        let mut state = self.sent.clone().filter(|s| s.run == run);
        let origin = batch.0[0].at.since_origin();
        let mut records = Vec::with_capacity(batch.0.len());
        for (i, m) in batch.0.iter().enumerate() {
            let at = (i > 0).then(|| {
                let off = m.at.since_origin().saturating_sub(origin);
                let scheduled = l.params.step_ms.is_some_and(|step| {
                    let instant = Duration::from_millis(u64::from(step) * i as u64);
                    let jitter = Duration::from_millis(u64::from(l.params.jitter_ms.unwrap_or(0)));
                    off.abs_diff(instant) <= jitter
                });
                if scheduled {
                    At::Scheduled
                } else {
                    At::Offset(u64::try_from(off.as_millis()).unwrap_or(u64::MAX))
                }
            });
            let (full, slots, next) = match state {
                None => (true, m.slots.clone(), None),
                Some(ref s) => {
                    let include: Vec<bool> =
                        (0..n).map(|k| self.includes(s, k, m.slots[k])).collect();
                    let full =
                        s.since_full + 1 >= l.params.keyframe_every || include.iter().all(|x| *x);
                    let slots = if full {
                        m.slots.clone()
                    } else {
                        m.slots
                            .iter()
                            .zip(&include)
                            .map(|(slot, inc)| if *inc { *slot } else { Slot::Omitted })
                            .collect()
                    };
                    (full, slots, Some(s))
                }
            };
            let mut s = next.cloned().unwrap_or(Sent {
                run,
                prev: m.slots.clone(),
                included: m.slots.clone(),
                since: vec![0; n],
                since_full: 0,
            });
            for (k, slot) in slots.iter().enumerate() {
                if matches!(slot, Slot::Omitted) {
                    s.since[k] = s.since[k].saturating_add(1);
                } else {
                    s.included[k] = *slot;
                    s.since[k] = 0;
                }
            }
            s.since_full = if full { 0 } else { s.since_full + 1 };
            s.prev = m.slots.clone();
            state = Some(s);
            records.push(Record { full, at, slots });
        }
        Encoded {
            frame: Frame {
                time: batch.0[0].wall,
                records,
            },
            next: state.expect("a record at least"),
        }
    }

    /// Whether a delta record after `s` includes field `k`, now `slot`.
    fn includes(&self, s: &Sent, k: usize, slot: Slot) -> bool {
        let Some(d) = self.layout.deadband(k) else {
            return slot != s.prev[k];
        };
        let (Slot::Code(now), Slot::Code(then)) = (slot, s.included[k]) else {
            return true;
        };
        if s.since[k] + 1 >= d.max_records {
            return true;
        }
        match (self.layout.weighed(k, now), self.layout.weighed(k, then)) {
            (Some(a), Some(b)) => a.distance(&b) > d.delta,
            _ => true,
        }
    }

    /// `e` was sent: the next frame is written against it.
    pub fn sent(&mut self, e: Encoded) {
        self.sent = Some(e.next);
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

    /// A frame its layout refuses is never applied (core §7.7): the next
    /// frame follows a gap that counts its records, and names it.
    #[test]
    fn a_refused_frame_reads_as_lost_and_is_named() {
        let l = Layout::of(&link(), &one()).unwrap();
        let stamp = |seq| Stamp {
            run: crate::wire::Run::U16(1),
            seq: Seq::new(seq).unwrap(),
        };
        let full = Frame::from_json(
            &json!({"time": {"s": 1, "ms": 0}, "records": [{"full": true, "values": {"dropped": 1}}]}),
            &l,
            None,
        )
        .unwrap();
        let mut r = Reader::new();
        assert!(r.apply(&l, stamp(10), &full).refused.is_empty());
        r.refuse(stamp(11));
        let next = r.apply(&l, stamp(12), &full);
        assert_eq!(next.follows, Follows::Gap { lost: 1 });
        assert_eq!(next.refused, [Seq::new(11).unwrap()]);
        // Noted once: the frame after reads none.
        assert!(r.apply(&l, stamp(13), &full).refused.is_empty());
        // Another run's refusal is forgotten.
        r.refuse(Stamp {
            run: crate::wire::Run::U16(2),
            seq: Seq::new(0).unwrap(),
        });
        assert!(r.apply(&l, stamp(14), &full).refused.is_empty());
    }

    fn sample() -> packed::Layout {
        let doc = json!({"types": {"Sample": {"fields": [
            {"name": "temperature", "type": "fixed", "minimum": "-40", "maximum": "85",
             "resolution": "0.5"},
            {"name": "rssi", "type": "int", "minimum": -140, "maximum": -20,
             "reserved": ["not_available"]},
            {"name": "link_up", "type": "bool"},
            {"name": "mode", "type": "enum", "values": ["idle", "rx", "tx"], "escape": true,
             "optional": true}
        ]}}});
        packed::layout_in(&doc, "Sample").expect("a layout")
    }

    fn params(spec: FrameSpec) -> FrameParams {
        FrameParams::resolve(&spec).expect("a table")
    }

    fn measured(l: &Layout, v: Value, ms: u64) -> Measured {
        Measured {
            slots: l.slots_of_json(&v, None).expect("slots"),
            at: crate::timing::Mono::after(Duration::from_millis(ms)),
            wall: Some(Time { s: 100, ms: 0 }),
        }
    }

    fn base(queue: u64) -> Value {
        json!({"dropped": 7, "queue_depth": queue, "duty_available": 1, "duty_wait_ms": "1500"})
    }

    /// Writes each batch under `run` from seq `next`, as an owner would,
    /// and reads it back through a reader.
    fn send(f: &mut Framer, r: &mut Reader, batch: &Batch, run: Run, next: &mut u32) -> Applied {
        let e = f.encode(batch, run);
        let l = f.layout().clone();
        let stamp = Stamp {
            run,
            seq: Seq::new(*next).unwrap(),
        };
        let put = e.frame.envelope(&l, stamp).expect("written");
        let env = Envelope::parse(&put).unwrap();
        let back = Frame::parse(env.tag, env.record, &l).expect("read back");
        assert_eq!(back, e.frame);
        *next += batch.len();
        f.sent(e);
        r.apply(&l, stamp, &back)
    }

    /// The owner's rules (core §7.7): its first frame in a run full, a full
    /// record every `keyframe_every`, a delta record including what
    /// changed, a deadband omitted within `delta` and included every
    /// `max_records`, and a new run full again; a reader of its frames
    /// reads every field known or within.
    #[test]
    fn a_framer_writes_the_owners_frames() {
        let p = params(FrameSpec {
            keyframe_every: Some(4),
            deadband: [(
                "queue_depth".to_owned(),
                crate::authoring::DeadbandSpec {
                    delta: "4".to_owned(),
                    max_records: 3,
                },
            )]
            .into(),
            ..FrameSpec::default()
        });
        let l = Layout::of(&link(), &p).unwrap();
        let mut f = Framer::new(l.clone());
        let mut r = Reader::new();
        let (run, mut next) = (Run::U16(1), 0);
        let one = |f: &mut Framer, r: &mut Reader, v: Value, ms: u64, run: Run, next: &mut u32| {
            let b = f
                .push(measured(&l, v, ms))
                .unwrap()
                .expect("one record a frame");
            send(f, r, &b, run, next)
        };
        let a = one(&mut f, &mut r, base(12), 0, run, &mut next);
        assert!(
            a.records[0]
                .fields
                .iter()
                .all(|s| matches!(s, FieldState::Known(_)))
        );
        // Within 4 of 12, the rest unchanged: nothing included.
        let a = one(&mut f, &mut r, base(15), 1000, run, &mut next);
        assert_eq!(a.follows, Follows::Continues);
        assert_eq!(a.records[0].fields[1], FieldState::Within(12));
        let a = one(&mut f, &mut r, base(9), 2000, run, &mut next);
        assert_eq!(a.records[0].fields[1], FieldState::Within(12));
        // Three records after its inclusion: included, 13.
        let a = one(&mut f, &mut r, base(13), 3000, run, &mut next);
        assert_eq!(a.records[0].fields[1], FieldState::Known(13));
        // Four records after the full one: full again.
        let mut v = base(13);
        v["duty_wait_ms"] = json!("2500");
        let e = f.encode(
            &f.clone()
                .push(measured(&l, v.clone(), 4000))
                .unwrap()
                .unwrap(),
            run,
        );
        assert!(e.frame.records[0].full);
        let a = one(&mut f, &mut r, v, 4000, run, &mut next);
        assert_eq!(a.records[0].fields[3], FieldState::Known(25));
        // A delta: the wait changed alone.
        let mut v = base(13);
        v["duty_wait_ms"] = json!("1500");
        let e = f.encode(
            &f.clone()
                .push(measured(&l, v.clone(), 5000))
                .unwrap()
                .unwrap(),
            run,
        );
        assert!(!e.frame.records[0].full);
        assert_eq!(
            e.frame.records[0].slots,
            vec![Slot::Omitted, Slot::Omitted, Slot::Omitted, Slot::Code(15)]
        );
        // A field turned absent is included, its presence bit 0.
        let a = one(&mut f, &mut r, v, 5000, run, &mut next);
        let mut v = base(13);
        v.as_object_mut().unwrap().remove("dropped");
        let a2 = one(&mut f, &mut r, v, 6000, run, &mut next);
        assert_eq!(a.records[0].fields[0], FieldState::Known(7));
        assert_eq!(a2.records[0].fields[0], FieldState::Absent);
        // A new run: full, whatever changed.
        let b = f.push(measured(&l, base(13), 7000)).unwrap().unwrap();
        let e = f.encode(&b, Run::U16(2));
        assert!(e.frame.records[0].full);
        // A frame never sent leaves nothing: the next is written against
        // the last one sent, `dropped` present again, and the queue three
        // records after its inclusion.
        let e = f.encode(&b, run);
        assert_eq!(
            e.frame.records[0].slots,
            vec![Slot::Code(7), Slot::Code(13), Slot::Omitted, Slot::Omitted]
        );
    }

    /// Times (core §7.7): a record within the jitter of its instant is
    /// scheduled, another measured; a record at or past the span ends the
    /// frame and opens the next; a record measured before the last is
    /// refused.
    #[test]
    fn a_framer_schedules_and_ends_frames() {
        let p = params(FrameSpec {
            records: Some(3),
            step_ms: Some(10_000),
            jitter_ms: Some(500),
            keyframe_every: Some(3),
            ..FrameSpec::default()
        });
        let l = Layout::of(&sample(), &p).unwrap();
        let mut f = Framer::new(l.clone());
        let v = |t: &str, rssi: i64, mode: &str| json!({"temperature": t, "rssi": rssi, "link_up": true, "mode": mode});
        assert_eq!(f.push(measured(&l, v("21.5", -80, "rx"), 0)).unwrap(), None);
        assert_eq!(
            f.push(measured(&l, v("21.5", -82, "rx"), 10_200)).unwrap(),
            None
        );
        let b = f
            .push(measured(&l, v("22", -82, "tx"), 20_700))
            .unwrap()
            .expect("three records");
        let e = f.encode(&b, Run::U16(1));
        let at: Vec<_> = e.frame.records.iter().map(|r| r.at).collect();
        assert_eq!(
            at,
            vec![None, Some(At::Scheduled), Some(At::Offset(20_700))]
        );
        assert_eq!(e.frame.records[1].slots[1], Slot::Code(58));
        assert!(matches!(e.frame.records[1].slots[0], Slot::Omitted));
        f.sent(e);
        assert_eq!(
            f.push(measured(&l, v("22", -82, "tx"), 30_100)).unwrap(),
            None
        );
        assert_eq!(
            f.push(measured(&l, v("22", -82, "tx"), 39_700)).unwrap(),
            None
        );
        let b = f
            .push(measured(&l, v("22", -82, "tx"), 65_100))
            .unwrap()
            .expect("past the span");
        assert_eq!(b.len(), 2);
        let e = f.encode(&b, Run::U16(1));
        assert!(e.frame.records[0].full, "three records after the last full");
        assert_eq!(e.frame.records[1].at, Some(At::Scheduled));
        assert_eq!(f.open(), 1);
        assert_eq!(
            f.push(measured(&l, v("22", -82, "tx"), 65_000)),
            Err(WriteRefusal::Order)
        );
        assert_eq!(f.flush().map(|b| b.len()), Some(1));
        assert_eq!(f.flush(), None);
        // A slot omitted, or a code its field does not use, is refused.
        let mut m = measured(&l, v("22", -82, "tx"), 70_000);
        m.slots[0] = Slot::Omitted;
        assert_eq!(f.push(m), Err(WriteRefusal::Value));
        let mut m = measured(&l, v("22", -82, "tx"), 70_000);
        m.slots[0] = Slot::Code(251);
        assert_eq!(f.push(m), Err(WriteRefusal::Range));
    }

    /// The largest frame (link.v1 §2.8): `records` full records with every
    /// optional field and every offset.
    #[test]
    fn the_largest_frame() {
        let l = Layout::of(&link(), &one()).unwrap();
        assert_eq!(
            (l.largest_body_bits(), l.largest_payload_bytes()),
            (106, 22)
        );
        let p = params(FrameSpec {
            records: Some(3),
            step_ms: Some(10_000),
            ..FrameSpec::default()
        });
        let l = Layout::of(&sample(), &p).unwrap();
        // 42 + 2 + 3 × (1 + 19) + 2 × (1 + 15).
        assert_eq!(l.largest_body_bits(), 136);
        let all = json!({"temperature": "85", "rssi": -20, "link_up": true, "mode": "tx"});
        let slots = l.slots_of_json(&all, None).unwrap();
        let frame = Frame {
            time: Some(Time { s: 1, ms: 2 }),
            records: (0..3)
                .map(|i| Record {
                    full: true,
                    at: (i > 0).then_some(At::Offset(29_999)),
                    slots: slots.clone(),
                })
                .collect(),
        };
        assert_eq!(frame.write(&l).unwrap().bits, l.largest_body_bits());
        // A record's bytes read into its slots as its JSON does.
        let record = sample().write(&all, None).unwrap();
        assert_eq!(l.slots_of(&record.bytes).unwrap(), slots);
        assert_eq!(Time::from_unix_ms(1_500, 1), Some(Time { s: 0, ms: 500 }));
        assert_eq!(Time::from_unix_ms(500, 1), None);
        assert_eq!(Time { s: 0, ms: 500 }.unix_ms(1), 1_500);
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

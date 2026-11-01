//! Packed records in Rust (core §7.6, #979 B2): what the packed types
//! `zenkey-build` generates are written against, and the runtime's
//! [`crate::codec::Packed`] carries.
//!
//! A generated type is a struct with a field per field of its record: an
//! `int` an `i64`, a `fixed` an exact [`Decimal`], an `enum` a generated
//! Rust enum ([`Open`] around it with an escape), a `bool`, a `counter` a
//! `u64`, a `time` an `i64` of milliseconds since the Unix epoch, a field
//! that declares reserved codes a [`Coded`], and an optional one, or one
//! after an extension marker, an `Option`. Its [`PackedRecord`] impl is
//! straight-line code, one statement per field in the layout's order,
//! built on this module's helpers, which are `zenkey_model::packed`'s: the
//! bits ([`BitWriter`], [`BitReader`]), exact fixed point ([`Fixed`]), a
//! time's code ([`time_code`]) and the refusals' tokens. It agrees with the
//! model's interpreted codec bit for bit, `spec/conformance/packed/vectors.json`
//! deciding (`zenkey-build/fixtures/codegen-test/tests/packed.rs`).
//!
//! Session-free: a contract crate's packed types build without zenoh.

pub use serde_json::{Map, Value as Json};
pub use zenkey_model::packed::{
    BitReader, BitWriter, DEFAULT_EPOCH, Decimal, Fixed, ReadRefusal, Record, WriteRefusal,
    fixed_code, fixed_value, int_code, int_value, json_decimal, json_integer, time_code,
    time_value, top_code,
};

/// A generated packed type (core §7.6): its layout's identity, its codec,
/// and core §7.6's JSON reading.
pub trait PackedRecord: Sized + Send + Sync + 'static {
    /// The type's name, its `types` key.
    const NAME: &'static str;
    /// The tag its envelope's CH carries: the low four bits of its layout
    /// form's sha256.
    const TAG: u8;
    /// The shortest and the longest record this revision writes, in bits
    /// before the pad.
    const MIN_BITS: u64;
    const MAX_BITS: u64;

    /// Writes the record: its bits, then the pad. `epoch` is the epoch the
    /// deployment states, in seconds, which a `time` field is written
    /// since: `None` where it states none, so E = 0 ([`DEFAULT_EPOCH`],
    /// core §7.6).
    ///
    /// # Errors
    /// The writer's refusal: a value with no code (`range`), a reserved
    /// code the field does not declare (`value`).
    fn encode(&self, epoch: Option<i64>) -> Result<Record, WriteRefusal>;

    /// Reads a record under the tag its envelope's CH carried, refusing it
    /// for the first of core §7.6's checks that applies, its `time` fields
    /// under `epoch` as [`PackedRecord::encode`] takes it.
    ///
    /// # Errors
    /// The reader's refusal.
    fn decode(tag: u8, bytes: &[u8], epoch: Option<i64>) -> Result<Self, ReadRefusal>;

    /// Core §7.6's JSON reading: a member per field present.
    fn to_json(&self) -> Json;

    /// The value core §7.6's JSON reading spells, a fixed-point value as a
    /// string or a number.
    ///
    /// # Errors
    /// The writer's refusal: not an object or a value of the wrong JSON
    /// type (`value`), a member the type does not have (`unknown`), a field
    /// that is not optional missing (`missing`), an integer past 64 bits
    /// (`range`). A value with no code is [`PackedRecord::encode`]'s to
    /// refuse.
    fn from_json(v: &Json) -> Result<Self, WriteRefusal>;
}

/// A field that declares reserved codes (core §7.6): a value, or one of
/// the codes `reserved` names. Writing a reserved code the field does not
/// declare is refused (`value`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Coded<T> {
    Value(T),
    /// `not_available`: all ones.
    NotAvailable,
    /// `error`: the code below all ones.
    Error,
    /// `out_of_range`: the code below that.
    OutOfRange,
}

impl<T> Coded<T> {
    /// The reserved code's name, `None` for a value.
    #[must_use]
    pub fn reserved(&self) -> Option<&'static str> {
        match self {
            Self::Value(_) => None,
            Self::NotAvailable => Some("not_available"),
            Self::Error => Some("error"),
            Self::OutOfRange => Some("out_of_range"),
        }
    }

    /// The value, `None` for a reserved code.
    #[must_use]
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Value(v) => Some(v),
            _ => None,
        }
    }

    /// A reserved code by its name, as core §7.6's JSON reading spells it.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "not_available" => Some(Self::NotAvailable),
            "error" => Some(Self::Error),
            "out_of_range" => Some(Self::OutOfRange),
            _ => None,
        }
    }
}

impl<T> From<T> for Coded<T> {
    fn from(v: T) -> Self {
        Self::Value(v)
    }
}

/// An enum with an escape (core §7.6): a value this revision names, or the
/// escape, a value it does not (`null` in the JSON reading). A reader reads
/// every code from the enum's count up as the escape, so a later revision
/// can name more within the width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Open<E> {
    Known(E),
    Escape,
}

impl<E> From<E> for Open<E> {
    fn from(v: E) -> Self {
        Self::Known(v)
    }
}

/// The members of a JSON object, or `value` (generated `from_json`).
///
/// # Errors
/// [`WriteRefusal::Value`].
pub fn object(v: &Json) -> Result<&Map<String, Json>, WriteRefusal> {
    v.as_object().ok_or(WriteRefusal::Value)
}

/// `unknown` for a member outside `names`.
///
/// # Errors
/// [`WriteRefusal::Unknown`].
pub fn known(m: &Map<String, Json>, names: &[&str]) -> Result<(), WriteRefusal> {
    if m.keys().all(|k| names.contains(&k.as_str())) {
        Ok(())
    } else {
        Err(WriteRefusal::Unknown)
    }
}

/// A field that is not optional: `missing` when absent.
///
/// # Errors
/// [`WriteRefusal::Missing`].
pub fn required<'a>(m: &'a Map<String, Json>, name: &str) -> Result<&'a Json, WriteRefusal> {
    m.get(name).ok_or(WriteRefusal::Missing)
}

/// An `int` value from JSON: an integer (`value` otherwise, `range` past 64
/// bits).
///
/// # Errors
/// [`WriteRefusal::Value`] or [`WriteRefusal::Range`].
pub fn json_i64(v: &Json) -> Result<i64, WriteRefusal> {
    i64::try_from(json_integer(v)?).map_err(|_| WriteRefusal::Range)
}

/// A `counter` value from JSON: a count from 0 to 2^64 − 1.
///
/// # Errors
/// [`WriteRefusal::Value`] or [`WriteRefusal::Range`].
pub fn json_u64(v: &Json) -> Result<u64, WriteRefusal> {
    u64::try_from(json_integer(v)?).map_err(|_| WriteRefusal::Range)
}

/// A `bool` value from JSON.
///
/// # Errors
/// [`WriteRefusal::Value`].
pub fn json_bool(v: &Json) -> Result<bool, WriteRefusal> {
    v.as_bool().ok_or(WriteRefusal::Value)
}

/// A field with reserved codes from JSON: a reserved code's name, else
/// `value` read by `read`.
///
/// # Errors
/// `read`'s refusal.
pub fn json_coded<T>(
    v: &Json,
    read: impl FnOnce(&Json) -> Result<T, WriteRefusal>,
) -> Result<Coded<T>, WriteRefusal> {
    match v.as_str().and_then(Coded::named) {
        Some(c) => Ok(c),
        None => read(v).map(Coded::Value),
    }
}

/// An `enum` value from JSON: a name `named` knows (`value` otherwise).
///
/// # Errors
/// [`WriteRefusal::Value`].
pub fn json_enum<E>(v: &Json, named: impl FnOnce(&str) -> Option<E>) -> Result<E, WriteRefusal> {
    v.as_str().and_then(named).ok_or(WriteRefusal::Value)
}

/// An enum with an escape from JSON: `null` is the escape.
///
/// # Errors
/// [`WriteRefusal::Value`].
pub fn json_open<E>(
    v: &Json,
    named: impl FnOnce(&str) -> Option<E>,
) -> Result<Open<E>, WriteRefusal> {
    if v.is_null() {
        Ok(Open::Escape)
    } else {
        json_enum(v, named).map(Open::Known)
    }
}

/// A `time` instant from its code: [`time_value`], in an `i64` of
/// milliseconds since the Unix epoch, under the deployment epoch (`None`:
/// [`DEFAULT_EPOCH`]).
///
/// # Errors
/// [`ReadRefusal::Code`], then [`ReadRefusal::Epoch`] for an epoch that
/// places the instant past an `i64`.
pub fn time_ms(code: u64, epoch: Option<i64>, ms: bool) -> Result<i64, ReadRefusal> {
    i64::try_from(time_value(code, epoch, ms)?).map_err(|_| ReadRefusal::Epoch)
}

/// An `int` value from its code: [`int_value`], in an `i64`.
///
/// # Errors
/// [`ReadRefusal::Code`].
pub fn int_i64(code: u64, minimum: i64, maximum: i64) -> Result<i64, ReadRefusal> {
    // Within [minimum, maximum], both i64.
    int_value(code, minimum, maximum).map(|v| i64::try_from(v).unwrap_or(maximum))
}

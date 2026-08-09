//! The `cdr` schema kind (issue #98) — dynamic CDR encode/decode for
//! DDS / ROS 2 interop, feature `decode-cdr`.
//!
//! CDR is the second **non-self-describing** encoding this crate can read, and
//! it is the one that proves the seam is general: `protobuf` arrived with a
//! descriptor format of its own, so "add a codec" could still have meant "add
//! protobuf". Here the schema document is defined by this convention, and
//! RFC 08 §7's kind vocabulary is open by its own rule — so registering `cdr`
//! is additive, and an older consumer skips it without losing the rest of the
//! set.
//!
//! ## The served document
//!
//! The canonical form is a **compact JSON field list**, not IDL or `.msg`
//! text. Two reasons: the hash is then JCS over a JSON value exactly like
//! `json-schema` (no second canonicalization story), and a producer-side
//! generator can emit it from either source language. The source text rides
//! along informatively so a human still sees what they wrote.
//!
//! ```json
//! {
//!   "fields": [ {"name": "linear", "type": "Vector3"},
//!               {"name": "angular", "type": "Vector3"} ],
//!   "types":  { "Vector3": { "fields": [ {"name": "x", "type": "float64"},
//!                                        {"name": "y", "type": "float64"},
//!                                        {"name": "z", "type": "float64"} ] } },
//!   "source": { "language": "ros2msg", "text": "Vector3 linear\nVector3 angular\n" }
//! }
//! ```
//!
//! A field's `type` is a primitive name (`bool`, `int8`…`uint64`, `float32`,
//! `float64`, `string`), a key into `types`, or one of the two composite
//! objects `{"array": {"of": …, "len": N}}` and
//! `{"sequence": {"of": …, "bound": N?}}`.
//!
//! ## What is implemented
//!
//! **XCDR1** (classic `PLAIN_CDR`): the 4-byte RTPS encapsulation header,
//! then primitives at their natural alignment *relative to the start of the
//! body*, `string` as a `uint32` length **including** its NUL terminator, and
//! `sequence` as a `uint32` count followed by its elements. Big-endian is
//! accepted on decode; encode always produces the canonical little-endian
//! encapsulation, which is what makes the round trip byte-identical.
//!
//! Alignment is applied at the **primitive**, never at the aggregate: XCDR1
//! structs and arrays add no padding of their own, so a struct is padded
//! exactly as much as its first primitive member demands. That is why the
//! reader and writer here carry an offset and nothing else — an aggregate
//! alignment rule would be padding this framing does not have.
//!
//! XCDR2 and appendable/mutable type evolution are deliberately out of scope
//! (RFC 08 §7 records why): they change the framing itself, and nothing in
//! this workspace has a peer that speaks them yet.

use serde_json::{Map, Value};

use super::decode::{DecodeError, DecodedPayload, PayloadDecoder};
use super::{SchemaKind, TypeSchema, WireEncoding};

/// How deep a `types` chain may nest before we call it a cycle. A schema is
/// foreign input; recursion on it must terminate on a bound, not on a stack.
const MAX_DEPTH: usize = 32;

/// A resolved CDR type — the schema document, interpreted.
#[derive(Debug, Clone, PartialEq)]
enum CdrType {
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    Str,
    /// Fixed-length array: elements only, no count on the wire.
    Array {
        of: Box<CdrType>,
        len: usize,
    },
    /// Sequence: `uint32` count, then elements. The declared bound is
    /// enforced on encode (a producer that declared one meant it).
    Sequence {
        of: Box<CdrType>,
        bound: Option<usize>,
    },
    Message(Vec<(String, CdrType)>),
}

fn bad_schema(msg: impl Into<String>) -> DecodeError {
    DecodeError::BadSchema(msg.into())
}

/// Resolve one `type` spelling against the document's `types` table.
fn resolve(spec: &Value, types: &Map<String, Value>, depth: usize) -> Result<CdrType, DecodeError> {
    if depth > MAX_DEPTH {
        return Err(bad_schema(format!(
            "type nesting exceeds {MAX_DEPTH} — a cycle in `types`?"
        )));
    }
    match spec {
        Value::String(name) => match name.as_str() {
            "bool" => Ok(CdrType::Bool),
            "int8" | "char" => Ok(CdrType::I8),
            "uint8" | "byte" | "octet" => Ok(CdrType::U8),
            "int16" | "short" => Ok(CdrType::I16),
            "uint16" | "unsigned short" => Ok(CdrType::U16),
            "int32" | "long" => Ok(CdrType::I32),
            "uint32" | "unsigned long" => Ok(CdrType::U32),
            "int64" | "long long" => Ok(CdrType::I64),
            "uint64" | "unsigned long long" => Ok(CdrType::U64),
            "float32" | "float" => Ok(CdrType::F32),
            "float64" | "double" => Ok(CdrType::F64),
            "string" => Ok(CdrType::Str),
            other => {
                let entry = types
                    .get(other)
                    .ok_or_else(|| bad_schema(format!("type {other:?} is not in `types`")))?;
                let fields = entry
                    .get("fields")
                    .ok_or_else(|| bad_schema(format!("type {other:?} declares no `fields`")))?;
                Ok(CdrType::Message(resolve_fields(fields, types, depth + 1)?))
            }
        },
        Value::Object(obj) if obj.contains_key("array") => {
            let a = obj["array"]
                .as_object()
                .ok_or_else(|| bad_schema("`array` must be an object"))?;
            let of = a
                .get("of")
                .ok_or_else(|| bad_schema("`array` needs `of`"))?;
            let len = a
                .get("len")
                .and_then(Value::as_u64)
                .ok_or_else(|| bad_schema("`array` needs a numeric `len`"))?;
            Ok(CdrType::Array {
                of: Box::new(resolve(of, types, depth + 1)?),
                len: len as usize,
            })
        }
        Value::Object(obj) if obj.contains_key("sequence") => {
            let s = obj["sequence"]
                .as_object()
                .ok_or_else(|| bad_schema("`sequence` must be an object"))?;
            let of = s
                .get("of")
                .ok_or_else(|| bad_schema("`sequence` needs `of`"))?;
            Ok(CdrType::Sequence {
                of: Box::new(resolve(of, types, depth + 1)?),
                bound: s.get("bound").and_then(Value::as_u64).map(|b| b as usize),
            })
        }
        other => Err(bad_schema(format!("unrecognised type spec {other}"))),
    }
}

fn resolve_fields(
    fields: &Value,
    types: &Map<String, Value>,
    depth: usize,
) -> Result<Vec<(String, CdrType)>, DecodeError> {
    let list = fields
        .as_array()
        .ok_or_else(|| bad_schema("`fields` must be an array"))?;
    let mut out = Vec::with_capacity(list.len());
    for f in list {
        let name = f
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| bad_schema("a field is missing `name`"))?;
        let spec = f
            .get("type")
            .ok_or_else(|| bad_schema(format!("field {name:?} is missing `type`")))?;
        out.push((name.to_string(), resolve(spec, types, depth)?));
    }
    Ok(out)
}

/// The top-level message a `cdr` entry describes.
fn resolve_schema(schema: &TypeSchema) -> Result<CdrType, DecodeError> {
    let fields = schema
        .cdr_fields()
        .ok_or_else(|| bad_schema("missing `fields`"))?;
    let empty = Map::new();
    let types = schema.cdr_types().unwrap_or(&empty);
    Ok(CdrType::Message(resolve_fields(fields, types, 0)?))
}

/// The 4-byte RTPS encapsulation header. `0x0000` = big-endian CDR,
/// `0x0001` = little-endian; the two option bytes are unused in XCDR1.
const ENCAPSULATION_LE: [u8; 4] = [0x00, 0x01, 0x00, 0x00];

struct Reader<'a> {
    body: &'a [u8],
    pos: usize,
    le: bool,
}

impl<'a> Reader<'a> {
    /// Split the encapsulation header off, and read its endianness.
    fn new(bytes: &'a [u8]) -> Result<Reader<'a>, DecodeError> {
        if bytes.len() < 4 {
            return Err(DecodeError::Malformed(
                "cdr",
                "payload is shorter than the 4-byte encapsulation header".into(),
            ));
        }
        let le = match (bytes[0], bytes[1]) {
            (0x00, 0x00) => false,
            (0x00, 0x01) => true,
            (a, b) => {
                return Err(DecodeError::Malformed(
                    "cdr",
                    format!(
                        "encapsulation {a:#04x}{b:02x} is not XCDR1 PLAIN_CDR \
                         (PL_CDR and XCDR2 are out of scope — RFC 08 §7)"
                    ),
                ));
            }
        };
        Ok(Reader {
            body: &bytes[4..],
            pos: 0,
            le,
        })
    }

    /// Alignment is relative to the **body**, which is why the header was
    /// split off rather than skipped over.
    fn align(&mut self, n: usize) {
        let rem = self.pos % n;
        if rem != 0 {
            self.pos += n - rem;
        }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).ok_or_else(|| {
            DecodeError::Malformed("cdr", "length overflows the address space".into())
        })?;
        if end > self.body.len() {
            return Err(DecodeError::Malformed(
                "cdr",
                format!(
                    "payload ends mid-value: wanted {n} bytes at offset {}, {} remain",
                    self.pos,
                    self.body.len().saturating_sub(self.pos)
                ),
            ));
        }
        let out = &self.body[self.pos..end];
        self.pos = end;
        Ok(out)
    }
}

macro_rules! read_scalar {
    ($r:expr, $t:ty) => {{
        const N: usize = std::mem::size_of::<$t>();
        $r.align(N);
        let bytes: [u8; N] = $r.take(N)?.try_into().expect("take returns N bytes");
        if $r.le {
            <$t>::from_le_bytes(bytes)
        } else {
            <$t>::from_be_bytes(bytes)
        }
    }};
}

fn read(ty: &CdrType, r: &mut Reader<'_>) -> Result<Value, DecodeError> {
    Ok(match ty {
        CdrType::Bool => {
            r.align(1);
            Value::Bool(r.take(1)?[0] != 0)
        }
        CdrType::I8 => Value::from(read_scalar!(r, i8)),
        CdrType::U8 => Value::from(read_scalar!(r, u8)),
        CdrType::I16 => Value::from(read_scalar!(r, i16)),
        CdrType::U16 => Value::from(read_scalar!(r, u16)),
        CdrType::I32 => Value::from(read_scalar!(r, i32)),
        CdrType::U32 => Value::from(read_scalar!(r, u32)),
        CdrType::I64 => Value::from(read_scalar!(r, i64)),
        CdrType::U64 => Value::from(read_scalar!(r, u64)),
        CdrType::F32 => number(f64::from(read_scalar!(r, f32)))?,
        CdrType::F64 => number(read_scalar!(r, f64))?,
        CdrType::Str => {
            let len = read_scalar!(r, u32) as usize;
            if len == 0 {
                return Err(DecodeError::Malformed(
                    "cdr",
                    "string length 0 — CDR counts the NUL terminator, so the minimum is 1".into(),
                ));
            }
            let raw = r.take(len)?;
            // The declared length includes the terminator; the value does not.
            let text = std::str::from_utf8(&raw[..len - 1])
                .map_err(|e| DecodeError::Malformed("cdr", e.to_string()))?;
            Value::String(text.to_string())
        }
        CdrType::Array { of, len } => {
            let mut out = Vec::with_capacity(*len);
            for _ in 0..*len {
                out.push(read(of, r)?);
            }
            Value::Array(out)
        }
        CdrType::Sequence { of, .. } => {
            let count = read_scalar!(r, u32) as usize;
            // Do not pre-allocate on a foreign count: a corrupt 4 GiB length
            // must fail on the byte budget, not on the allocator.
            let mut out = Vec::new();
            for _ in 0..count {
                out.push(read(of, r)?);
            }
            Value::Array(out)
        }
        CdrType::Message(fields) => {
            let mut obj = Map::new();
            for (name, t) in fields {
                obj.insert(name.clone(), read(t, r)?);
            }
            Value::Object(obj)
        }
    })
}

/// JSON has no spelling for NaN or infinity, and inventing one (`null`,
/// `"NaN"`) would make the round trip lossy in silence.
fn number(v: f64) -> Result<Value, DecodeError> {
    serde_json::Number::from_f64(v)
        .map(Value::Number)
        .ok_or_else(|| {
            DecodeError::Malformed("cdr", format!("{v} has no JSON representation (NaN/inf)"))
        })
}

struct Writer {
    body: Vec<u8>,
}

impl Writer {
    fn align(&mut self, n: usize) {
        let rem = self.body.len() % n;
        if rem != 0 {
            // Padding is zero-filled: two encoders that pad differently would
            // produce different bytes for the same value, and the byte-identity
            // round trip is the guard on this whole module.
            self.body.resize(self.body.len() + (n - rem), 0);
        }
    }
}

macro_rules! write_scalar {
    ($w:expr, $t:ty, $v:expr) => {{
        const N: usize = std::mem::size_of::<$t>();
        $w.align(N);
        $w.body.extend_from_slice(&<$t>::to_le_bytes($v));
    }};
}

fn encode_err(msg: impl Into<String>) -> DecodeError {
    DecodeError::Encode(msg.into())
}

fn int<T>(v: &Value, name: &str) -> Result<T, DecodeError>
where
    T: TryFrom<i64>,
{
    let n = v
        .as_i64()
        .ok_or_else(|| encode_err(format!("{v} is not an integer ({name} expected)")))?;
    T::try_from(n).map_err(|_| encode_err(format!("{n} does not fit in {name}")))
}

fn write(ty: &CdrType, v: &Value, w: &mut Writer) -> Result<(), DecodeError> {
    match ty {
        CdrType::Bool => {
            let b = v
                .as_bool()
                .ok_or_else(|| encode_err(format!("{v} is not a bool")))?;
            w.align(1);
            w.body.push(u8::from(b));
        }
        CdrType::I8 => write_scalar!(w, i8, int::<i8>(v, "int8")?),
        CdrType::U8 => write_scalar!(w, u8, int::<u8>(v, "uint8")?),
        CdrType::I16 => write_scalar!(w, i16, int::<i16>(v, "int16")?),
        CdrType::U16 => write_scalar!(w, u16, int::<u16>(v, "uint16")?),
        CdrType::I32 => write_scalar!(w, i32, int::<i32>(v, "int32")?),
        CdrType::U32 => write_scalar!(w, u32, int::<u32>(v, "uint32")?),
        CdrType::I64 => write_scalar!(w, i64, int::<i64>(v, "int64")?),
        CdrType::U64 => {
            let n = v
                .as_u64()
                .ok_or_else(|| encode_err(format!("{v} is not a uint64")))?;
            write_scalar!(w, u64, n);
        }
        CdrType::F32 => {
            let n = v
                .as_f64()
                .ok_or_else(|| encode_err(format!("{v} is not a number (float32 expected)")))?;
            write_scalar!(w, f32, n as f32);
        }
        CdrType::F64 => {
            let n = v
                .as_f64()
                .ok_or_else(|| encode_err(format!("{v} is not a number (float64 expected)")))?;
            write_scalar!(w, f64, n);
        }
        CdrType::Str => {
            let s = v
                .as_str()
                .ok_or_else(|| encode_err(format!("{v} is not a string")))?;
            let len = u32::try_from(s.len() + 1)
                .map_err(|_| encode_err("string longer than a uint32 length"))?;
            write_scalar!(w, u32, len);
            w.body.extend_from_slice(s.as_bytes());
            w.body.push(0);
        }
        CdrType::Array { of, len } => {
            let items = v
                .as_array()
                .ok_or_else(|| encode_err(format!("{v} is not an array")))?;
            if items.len() != *len {
                return Err(encode_err(format!(
                    "array declares {len} elements, value has {}",
                    items.len()
                )));
            }
            for item in items {
                write(of, item, w)?;
            }
        }
        CdrType::Sequence { of, bound } => {
            let items = v
                .as_array()
                .ok_or_else(|| encode_err(format!("{v} is not an array")))?;
            if let Some(bound) = bound
                && items.len() > *bound
            {
                return Err(encode_err(format!(
                    "sequence is bounded at {bound}, value has {}",
                    items.len()
                )));
            }
            let count = u32::try_from(items.len())
                .map_err(|_| encode_err("sequence longer than a uint32 count"))?;
            write_scalar!(w, u32, count);
            for item in items {
                write(of, item, w)?;
            }
        }
        CdrType::Message(fields) => {
            let obj = v
                .as_object()
                .ok_or_else(|| encode_err(format!("{v} is not an object")))?;
            for (name, t) in fields {
                let field = obj.get(name).ok_or_else(|| {
                    encode_err(format!("missing field {name:?} — CDR has no absent fields"))
                })?;
                write(t, field, w)?;
            }
        }
    }
    Ok(())
}

/// Fields present in the value that the schema does not declare — visible,
/// never fatal, exactly as the `json-schema` codec treats them (RFC 08 §3's
/// additive-evolution posture). CDR itself cannot carry them, so this is a
/// note about what was *dropped*.
fn undeclared(fields: &[(String, CdrType)], value: &Value) -> Vec<String> {
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };
    obj.keys()
        .filter(|k| !fields.iter().any(|(n, _)| n == *k))
        .map(|k| format!("field {k:?} is not in the served schema and was not encoded"))
        .collect()
}

/// The `cdr` codec.
pub struct CdrDecoder;

impl PayloadDecoder for CdrDecoder {
    fn kind(&self) -> &str {
        SchemaKind::CDR
    }

    fn decode(
        &self,
        schema: &TypeSchema,
        encoding: &WireEncoding,
        bytes: &[u8],
    ) -> Result<DecodedPayload, DecodeError> {
        match encoding {
            // `Other` covers a bus that labels nothing: the schema says CDR,
            // and an unlabelled sample is "unsaid", not "not CDR".
            WireEncoding::Cdr | WireEncoding::Other(_) => {}
            other => return Err(DecodeError::WrongEncoding(format!("{other:?}"))),
        }
        let ty = resolve_schema(schema)?;
        let mut reader = Reader::new(bytes)?;
        let value = read(&ty, &mut reader)?;
        // Trailing bytes mean the schema and the payload disagree about the
        // message — reporting the prefix as "the value" is how a decoder
        // silently corrupts what it was asked to display.
        let mut notes = Vec::new();
        if reader.pos < reader.body.len() {
            notes.push(format!(
                "{} trailing byte(s) the schema does not account for",
                reader.body.len() - reader.pos
            ));
        }
        Ok(DecodedPayload { value, notes })
    }

    fn encode(
        &self,
        schema: &TypeSchema,
        value: &Value,
        _target: &WireEncoding,
    ) -> Result<Vec<u8>, DecodeError> {
        let ty = resolve_schema(schema)?;
        let mut w = Writer { body: Vec::new() };
        write(&ty, value, &mut w)?;
        let mut out = Vec::with_capacity(4 + w.body.len());
        out.extend_from_slice(&ENCAPSULATION_LE);
        out.extend_from_slice(&w.body);
        Ok(out)
    }
}

/// The notes a caller would get from encoding this value — dropped fields.
/// Exposed because `encode` returns bytes, and a silently narrowed payload is
/// exactly the thing this convention refuses to do.
pub fn encode_notes(schema: &TypeSchema, value: &Value) -> Vec<String> {
    match resolve_schema(schema) {
        Ok(CdrType::Message(fields)) => undeclared(&fields, value),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `geometry_msgs/Twist`, the canonical ROS 2 shape: a struct of structs
    /// of doubles.
    fn twist() -> TypeSchema {
        TypeSchema::cdr(json!({
            "fields": [
                {"name": "linear",  "type": "Vector3"},
                {"name": "angular", "type": "Vector3"}
            ],
            "types": {
                "Vector3": { "fields": [
                    {"name": "x", "type": "float64"},
                    {"name": "y", "type": "float64"},
                    {"name": "z", "type": "float64"}
                ]}
            },
            "source": {"language": "ros2msg", "text": "Vector3 linear\nVector3 angular\n"}
        }))
    }

    fn twist_value() -> Value {
        json!({
            "linear":  {"x": 1.0, "y": 0.0, "z": 0.0},
            "angular": {"x": 0.0, "y": 0.0, "z": 0.5}
        })
    }

    #[test]
    fn a_twist_round_trips_byte_identically() {
        let codec = CdrDecoder;
        let schema = twist();
        let value = twist_value();
        let bytes = codec
            .encode(&schema, &value, &WireEncoding::Cdr)
            .expect("encode");
        // 4-byte LE encapsulation + six f64s, all naturally aligned.
        assert_eq!(bytes.len(), 4 + 6 * 8);
        assert_eq!(&bytes[..4], &ENCAPSULATION_LE);

        let decoded = codec
            .decode(&schema, &WireEncoding::Cdr, &bytes)
            .expect("decode");
        assert_eq!(decoded.value, value);
        assert!(decoded.notes.is_empty());

        let again = codec
            .encode(&schema, &decoded.value, &WireEncoding::Cdr)
            .expect("re-encode");
        assert_eq!(again, bytes, "re-encoding must be byte-identical");
    }

    /// Alignment is the thing a hand-rolled CDR reader gets wrong. A
    /// `uint8` followed by a `uint32` must skip three bytes, and those bytes
    /// must be zeros.
    #[test]
    fn padding_is_inserted_and_is_zero() {
        let schema = TypeSchema::cdr(json!({
            "fields": [
                {"name": "flag",  "type": "uint8"},
                {"name": "count", "type": "uint32"},
                {"name": "wide",  "type": "uint64"}
            ]
        }));
        let value = json!({"flag": 1, "count": 2, "wide": 3});
        let bytes = CdrDecoder
            .encode(&schema, &value, &WireEncoding::Cdr)
            .expect("encode");
        assert_eq!(
            bytes,
            vec![
                0x00, 0x01, 0x00, 0x00, // encapsulation
                0x01, 0x00, 0x00, 0x00, // flag + 3 pad
                0x02, 0x00, 0x00, 0x00, // count (aligned 4)
                0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // wide (aligned 8)
            ]
        );
        assert_eq!(
            CdrDecoder
                .decode(&schema, &WireEncoding::Cdr, &bytes)
                .unwrap()
                .value,
            value
        );
    }

    /// Strings carry a length that counts the terminator, and sequences a
    /// count that does not.
    #[test]
    fn strings_and_sequences_use_their_declared_framings() {
        let schema = TypeSchema::cdr(json!({
            "fields": [
                {"name": "name",   "type": "string"},
                {"name": "values", "type": {"sequence": {"of": "int32"}}},
                {"name": "fixed",  "type": {"array": {"of": "uint8", "len": 3}}}
            ]
        }));
        let value = json!({"name": "hi", "values": [1, -2], "fixed": [7, 8, 9]});
        let bytes = CdrDecoder
            .encode(&schema, &value, &WireEncoding::Cdr)
            .unwrap();
        assert_eq!(
            &bytes[4..12],
            // uint32 length 3 ("hi" + NUL), then the bytes.
            &[0x03, 0x00, 0x00, 0x00, b'h', b'i', 0x00, 0x00]
        );
        let back = CdrDecoder
            .decode(&schema, &WireEncoding::Cdr, &bytes)
            .unwrap();
        assert_eq!(back.value, value);
    }

    /// A big-endian peer is readable; we just never write one.
    #[test]
    fn big_endian_encapsulation_decodes() {
        let schema = TypeSchema::cdr(json!({
            "fields": [{"name": "n", "type": "uint32"}]
        }));
        let be = vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00];
        let decoded = CdrDecoder
            .decode(&schema, &WireEncoding::Cdr, &be)
            .expect("big-endian decode");
        assert_eq!(decoded.value, json!({"n": 256}));
        // …and re-encoding normalises to the canonical little-endian form.
        let le = CdrDecoder
            .encode(&schema, &decoded.value, &WireEncoding::Cdr)
            .unwrap();
        assert_eq!(le, vec![0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00]);
    }

    /// Out-of-scope encapsulations fail with the reason, not with garbage.
    #[test]
    fn xcdr2_and_parameter_lists_are_refused_by_name() {
        let schema = TypeSchema::cdr(json!({"fields": []}));
        for (a, b) in [(0x00u8, 0x02u8), (0x00, 0x03), (0x00, 0x09)] {
            let err = CdrDecoder
                .decode(&schema, &WireEncoding::Cdr, &[a, b, 0, 0])
                .unwrap_err()
                .to_string();
            assert!(err.contains("XCDR1"), "{err}");
        }
    }

    /// A truncated payload is an error, never a partial value silently
    /// presented as the whole one.
    #[test]
    fn a_short_payload_names_where_it_ran_out() {
        let schema = TypeSchema::cdr(json!({
            "fields": [{"name": "wide", "type": "uint64"}]
        }));
        let err = CdrDecoder
            .decode(&schema, &WireEncoding::Cdr, &[0x00, 0x01, 0x00, 0x00, 0x01])
            .unwrap_err()
            .to_string();
        assert!(err.contains("ends mid-value"), "{err}");
    }

    /// Trailing bytes are reported rather than dropped: the schema and the
    /// payload disagree, and the user is the one who can tell why.
    #[test]
    fn trailing_bytes_are_a_note_not_a_silent_truncation() {
        let schema = TypeSchema::cdr(json!({
            "fields": [{"name": "n", "type": "uint32"}]
        }));
        let mut bytes = CdrDecoder
            .encode(&schema, &json!({"n": 1}), &WireEncoding::Cdr)
            .unwrap();
        bytes.push(0xff);
        let out = CdrDecoder
            .decode(&schema, &WireEncoding::Cdr, &bytes)
            .unwrap();
        assert_eq!(out.value, json!({"n": 1}));
        assert_eq!(out.notes.len(), 1);
        assert!(out.notes[0].contains("trailing"));
    }

    /// The declared bound means what it says, and a missing field is not
    /// silently zero-filled — CDR has no absent fields to encode.
    #[test]
    fn declared_shape_is_enforced_on_encode() {
        let schema = TypeSchema::cdr(json!({
            "fields": [{"name": "v", "type": {"sequence": {"of": "uint8", "bound": 2}}}]
        }));
        let err = CdrDecoder
            .encode(&schema, &json!({"v": [1, 2, 3]}), &WireEncoding::Cdr)
            .unwrap_err()
            .to_string();
        assert!(err.contains("bounded at 2"), "{err}");

        let err = CdrDecoder
            .encode(&schema, &json!({}), &WireEncoding::Cdr)
            .unwrap_err()
            .to_string();
        assert!(err.contains("missing field"), "{err}");
    }

    /// Undeclared fields cannot ride CDR at all, so the caller is told what
    /// was left behind instead of discovering it on the far end.
    #[test]
    fn dropped_fields_are_reportable() {
        let schema = TypeSchema::cdr(json!({
            "fields": [{"name": "n", "type": "uint32"}]
        }));
        let notes = encode_notes(&schema, &json!({"n": 1, "extra": true}));
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("\"extra\""));
    }

    /// A `types` cycle terminates on the depth bound rather than the stack.
    #[test]
    fn a_recursive_type_is_refused() {
        let schema = TypeSchema::cdr(json!({
            "fields": [{"name": "node", "type": "Node"}],
            "types": {"Node": {"fields": [{"name": "next", "type": "Node"}]}}
        }));
        let err = CdrDecoder
            .decode(&schema, &WireEncoding::Cdr, &[0x00, 0x01, 0x00, 0x00])
            .unwrap_err()
            .to_string();
        assert!(err.contains("cycle"), "{err}");
    }

    /// The hash covers the schema, not the informative source text: two
    /// producers generating the same message from `.msg` and from IDL agree.
    #[test]
    fn the_hash_ignores_the_informative_source() {
        let a = twist();
        let b = TypeSchema::cdr(json!({
            "fields": [
                {"name": "linear",  "type": "Vector3"},
                {"name": "angular", "type": "Vector3"}
            ],
            "types": {
                "Vector3": { "fields": [
                    {"name": "x", "type": "float64"},
                    {"name": "y", "type": "float64"},
                    {"name": "z", "type": "float64"}
                ]}
            },
            "source": {"language": "idl", "text": "struct Twist { Vector3 linear; … };"}
        }));
        assert_eq!(a.hash(), b.hash());
        // …but a real shape change moves it.
        let c = TypeSchema::cdr(json!({"fields": [{"name": "linear", "type": "float64"}]}));
        assert_ne!(a.hash(), c.hash());
    }
}

//! How a typed value becomes a payload and back (spec §7.2, #611).
//!
//! A [`Codec`] names a Rust type and how it is carried: [`Protobuf`] for a
//! prost message, [`Json`] for a serde type of a JSON Schema type (JSON or
//! CBOR, as the contract's `encoding` says), [`Raw`] for the bytes of a raw
//! media type. Generated code picks the codec from the contract; the wire
//! encoding is never the caller's choice. The typed handles
//! ([`crate::typed`]) are generic over it, so the encoding logic lives here
//! once.
//!
//! The decode order is §7.2's: the sample's `Encoding` first
//! ([`wire_of`]), then the contract's. A codec never evaluates a JSON
//! Schema: the Rust type is the validator.

use std::marker::PhantomData;

use serde::Serialize;
use serde::de::DeserializeOwned;
use zenkey_model::authoring::Encoding as WireEncoding;
use zenkey_model::envelope::Detail;
use zenkey_model::schema::TypeId;

/// What kind of contract type a codec carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecKind {
    Protobuf,
    JsonSchema,
    Raw,
    /// No type at all: a `summary` the operation does not declare.
    Nothing,
}

impl CodecKind {
    /// Whether a value of this codec is one of type `ty`; `None` is a type
    /// the contract does not declare.
    #[must_use]
    pub fn carries(self, ty: Option<&TypeId>) -> bool {
        matches!(
            (self, ty),
            (Self::Protobuf, Some(TypeId::Protobuf { .. }))
                | (Self::JsonSchema, Some(TypeId::JsonSchema { .. }))
                | (Self::Raw, Some(TypeId::Raw { .. }))
                | (Self::Nothing, None)
        )
    }
}

/// A Rust type and how it is carried on the wire.
pub trait Codec: Send + Sync + 'static {
    /// The Rust type.
    type Value: Send + 'static;
    /// The contract type kind it carries.
    const KIND: CodecKind;
    /// Encodes a value; `wire` is the contract's encoding for a JSON Schema
    /// type (JSON when `None`), ignored otherwise.
    fn encode(value: &Self::Value, wire: Option<WireEncoding>) -> Result<Vec<u8>, String>;
    /// Decodes a payload; `wire` is the effective encoding ([`wire_of`]).
    fn decode(bytes: &[u8], wire: Option<WireEncoding>) -> Result<Self::Value, String>;
    /// The `detail` of an `app` error envelope carrying this value (§5.2).
    fn detail(value: &Self::Value) -> Result<Detail, String> {
        Self::encode(value, None).map(Detail::Bytes)
    }
}

/// A prost message: `application/protobuf` (§7.2). `()` is
/// `google.protobuf.Empty`, as prost maps it.
pub struct Protobuf<T>(PhantomData<fn() -> T>);

impl<T: prost::Message + Default + 'static> Codec for Protobuf<T> {
    type Value = T;
    const KIND: CodecKind = CodecKind::Protobuf;
    fn encode(value: &T, _: Option<WireEncoding>) -> Result<Vec<u8>, String> {
        Ok(value.encode_to_vec())
    }
    fn decode(bytes: &[u8], _: Option<WireEncoding>) -> Result<T, String> {
        T::decode(bytes).map_err(|e| format!("protobuf: {e}"))
    }
}

/// A serde type for a JSON Schema type: JSON, or CBOR when the contract
/// says `encoding = "cbor"` (§7.2).
pub struct Json<T>(PhantomData<fn() -> T>);

impl<T: Serialize + DeserializeOwned + Send + 'static> Codec for Json<T> {
    type Value = T;
    const KIND: CodecKind = CodecKind::JsonSchema;
    fn encode(value: &T, wire: Option<WireEncoding>) -> Result<Vec<u8>, String> {
        encode_json(value, wire)
    }
    fn decode(bytes: &[u8], wire: Option<WireEncoding>) -> Result<T, String> {
        decode_json(bytes, wire)
    }
    fn detail(value: &T) -> Result<Detail, String> {
        serde_json::to_value(value)
            .map(Detail::Value)
            .map_err(|e| format!("JSON: {e}"))
    }
}

/// The bytes of a raw media type, as they are. The typed writers keep the
/// untyped one reachable for zero-copy payloads (SHM, §7.4).
pub struct Raw;

impl Codec for Raw {
    type Value = Vec<u8>;
    const KIND: CodecKind = CodecKind::Raw;
    fn encode(value: &Vec<u8>, _: Option<WireEncoding>) -> Result<Vec<u8>, String> {
        Ok(value.clone())
    }
    fn decode(bytes: &[u8], _: Option<WireEncoding>) -> Result<Vec<u8>, String> {
        Ok(bytes.to_vec())
    }
    /// A raw `error` type's detail is its bytes as base64 text (RFC 4648
    /// §4, padded) in the JSON envelope a raw type takes (§5.2, 0.7, F-65):
    /// bytes would not fit it, and would go out as `internal`.
    fn detail(value: &Vec<u8>) -> Result<Detail, String> {
        Ok(Detail::raw(value))
    }
}

/// The type of a `summary` an operation does not declare: nothing encodes
/// or decodes as it.
pub struct Nothing;

impl Codec for Nothing {
    type Value = ();
    const KIND: CodecKind = CodecKind::Nothing;
    fn encode(_: &(), _: Option<WireEncoding>) -> Result<Vec<u8>, String> {
        Err("this operation declares no such type".into())
    }
    fn decode(_: &[u8], _: Option<WireEncoding>) -> Result<(), String> {
        Err("this operation declares no such type".into())
    }
}

/// A value of a JSON Schema type, in its wire encoding (§7.2).
pub(crate) fn encode_json<T: Serialize>(
    value: &T,
    wire: Option<WireEncoding>,
) -> Result<Vec<u8>, String> {
    match wire {
        Some(WireEncoding::Cbor) => {
            let mut out = Vec::new();
            ciborium::into_writer(value, &mut out).map_err(|e| format!("CBOR: {e}"))?;
            Ok(out)
        }
        _ => serde_json::to_vec(value).map_err(|e| format!("JSON: {e}")),
    }
}

/// A JSON Schema value, decoded as JSON or CBOR.
pub(crate) fn decode_json<T: DeserializeOwned>(
    bytes: &[u8],
    wire: Option<WireEncoding>,
) -> Result<T, String> {
    if wire == Some(WireEncoding::Cbor) {
        ciborium::from_reader(bytes).map_err(|e| format!("CBOR: {e}"))
    } else {
        serde_json::from_slice(bytes).map_err(|e| format!("JSON: {e}"))
    }
}

/// The effective wire encoding of a JSON Schema payload (§7.2's decode
/// order): the sample's `Encoding`, when it says JSON or CBOR, else the
/// contract's.
#[must_use]
pub fn wire_of(sample: Option<&str>, contract: Option<WireEncoding>) -> Option<WireEncoding> {
    match sample {
        Some(e) if e.starts_with("application/cbor") => Some(WireEncoding::Cbor),
        Some(e) if e.starts_with("application/json") => Some(WireEncoding::Json),
        _ => contract,
    }
}

#[cfg(test)]
mod tests {
    use zenkey_model::authoring::Encoding as WireEncoding;

    use super::{Codec, Json, Protobuf, Raw, wire_of};

    #[test]
    fn codecs_round_trip_in_the_contracts_encoding() {
        let v = serde_json::json!({"a": [1, 2]});
        for wire in [None, Some(WireEncoding::Json), Some(WireEncoding::Cbor)] {
            let b = Json::<serde_json::Value>::encode(&v, wire).unwrap();
            assert_eq!(Json::<serde_json::Value>::decode(&b, wire).unwrap(), v);
        }
        let cbor = Json::<serde_json::Value>::encode(&v, Some(WireEncoding::Cbor)).unwrap();
        assert!(Json::<serde_json::Value>::decode(&cbor, None).is_err());
        assert_eq!(
            wire_of(Some("application/cbor"), Some(WireEncoding::Json)),
            Some(WireEncoding::Cbor)
        );
        assert_eq!(
            wire_of(Some("zenoh/bytes"), Some(WireEncoding::Cbor)),
            Some(WireEncoding::Cbor)
        );
        assert_eq!(Protobuf::<()>::encode(&(), None).unwrap(), Vec::<u8>::new());
        assert_eq!(Raw::decode(&[1, 2], None).unwrap(), [1, 2]);
    }

    /// Spec §5.2 (0.7, F-65): each codec's `app` detail has the form its
    /// envelope takes: a raw type's is base64 text, never bytes.
    #[test]
    fn a_raw_detail_is_base64_text() {
        use zenkey_model::envelope::Detail;
        let d = Raw::detail(&vec![0xff, 0xd8, 0xff, 0xe0]).unwrap();
        assert_eq!(d, Detail::Value(serde_json::json!("/9j/4A==")));
        assert_eq!(d.raw_bytes(), Some(vec![0xff, 0xd8, 0xff, 0xe0]));
        assert_eq!(
            Protobuf::<()>::detail(&()).unwrap(),
            Detail::Bytes(Vec::new())
        );
        assert_eq!(
            Json::<u32>::detail(&7).unwrap(),
            Detail::Value(serde_json::json!(7))
        );
    }
}

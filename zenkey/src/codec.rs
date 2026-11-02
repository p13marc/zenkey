//! How a typed value becomes a payload and back (spec §7.2, #611).
//!
//! A [`Codec`] names a Rust type and how it is carried: [`Protobuf`] for a
//! prost message, [`Json`] for a serde type of a JSON Schema type (JSON or
//! CBOR, as the contract's `encoding` says), [`Packed`] for a generated
//! packed type (core §7.6), [`Raw`] for the bytes of a raw media type. Generated code picks the codec from the contract; the wire
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
use zenkey_model::catalog::EPOCH_WITHOUT_CATALOG;
use zenkey_model::envelope::Detail;
use zenkey_model::schema::TypeId;
use zenkey_model::wire::Tag;

use crate::packed::PackedRecord;

/// What kind of contract type a codec carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecKind {
    Protobuf,
    JsonSchema,
    /// A packed type (core §7.6).
    Packed,
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
                | (Self::Packed, Some(TypeId::Packed { .. }))
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
    /// Decodes an envelope's record whose CH carried `tag` (core §7.5),
    /// `None` for a payload that came in no envelope. Only a packed type's
    /// record is read by its tag (§7.6); every other codec decodes as
    /// [`Codec::decode`] does.
    fn decode_record(
        bytes: &[u8],
        wire: Option<WireEncoding>,
        tag: Option<Tag>,
    ) -> Result<Self::Value, String> {
        let _ = tag;
        Self::decode(bytes, wire)
    }
    /// Encodes a value whose writer counts `time` fields from `epoch`, the
    /// deployment epoch in seconds (core §7.6, §8.6): only a packed type's
    /// record holds one, and every other codec encodes as
    /// [`Codec::encode`] does.
    fn encode_in(
        value: &Self::Value,
        wire: Option<WireEncoding>,
        epoch: i64,
    ) -> Result<Vec<u8>, String> {
        let _ = epoch;
        Self::encode(value, wire)
    }
    /// [`Codec::decode_record`], its `time` fields counted from `epoch`
    /// (core §7.6, §8.6).
    fn decode_record_in(
        bytes: &[u8],
        wire: Option<WireEncoding>,
        tag: Option<Tag>,
        epoch: i64,
    ) -> Result<Self::Value, String> {
        let _ = epoch;
        Self::decode_record(bytes, wire, tag)
    }
    /// Whether a value of this codec is one of type `ty`, the contract's:
    /// the kinds match ([`CodecKind::carries`]), and for a packed type its
    /// name too.
    fn carries(ty: Option<&TypeId>) -> bool {
        Self::KIND.carries(ty)
    }
    /// The tag its records carry in CH (core §7.6): a packed type's, `None`
    /// for every other codec, whose records carry tag 0.
    fn tag() -> Option<Tag> {
        None
    }
    /// The `detail` of an `app` error envelope carrying this value (§5.2).
    fn detail(value: &Self::Value) -> Result<Detail, String> {
        Self::encode(value, None).map(Detail::Bytes)
    }
    /// Core §7.6's JSON reading of a packed record, which keeps each
    /// `counter` field's count before its reduction modulo the field's
    /// width: a frame writer meters the count (`telemetry.v2` §2.19).
    /// `None` for every other codec.
    fn json_reading(value: &Self::Value) -> Option<serde_json::Value> {
        let _ = value;
        None
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

/// A generated packed type (core §7.6, [`crate::packed`]): its record, the
/// layout's bits and pad, which the writer seals in §7.5's envelope with
/// the type's tag in CH. Its `time` fields count from the deployment epoch
/// its writer or reader is given ([`Codec::encode_in`],
/// [`Codec::decode_record_in`]): the catalog's, where its service holds
/// one ([`crate::ServiceBuilder::catalog`]), else 0, the Unix epoch (core
/// §8.6). [`Codec::encode`] and [`Codec::decode`] read them under 0.
pub struct Packed<T>(PhantomData<fn() -> T>);

impl<T: PackedRecord> Packed<T> {
    fn record(tag: Option<Tag>, bytes: &[u8], epoch: i64) -> Result<T, String> {
        let Some(tag) = tag else {
            return Err(format!(
                "packed:{}: a packed record travels in an envelope, and this one came in none",
                T::NAME
            ));
        };
        T::decode(tag.get(), bytes, Some(epoch)).map_err(|r| format!("packed:{}: {r}", T::NAME))
    }
}

impl<T: PackedRecord> Codec for Packed<T> {
    type Value = T;
    const KIND: CodecKind = CodecKind::Packed;
    fn encode(value: &T, wire: Option<WireEncoding>) -> Result<Vec<u8>, String> {
        Self::encode_in(value, wire, EPOCH_WITHOUT_CATALOG)
    }
    /// A record read under the type's own tag, which checks no tag CH
    /// carried. A packed type travels only behind a CH (E044), and every
    /// typed read of one (a subscription, S4's GET, an archive's
    /// last-known) decodes by [`Codec::decode_record_in`] with the tag the
    /// envelope carried; this is for bytes a caller already admitted by it
    /// (`zenkey::inband::admit`).
    fn decode(bytes: &[u8], _: Option<WireEncoding>) -> Result<T, String> {
        T::decode(T::TAG, bytes, Some(EPOCH_WITHOUT_CATALOG))
            .map_err(|r| format!("packed:{}: {r}", T::NAME))
    }
    fn decode_record(
        bytes: &[u8],
        wire: Option<WireEncoding>,
        tag: Option<Tag>,
    ) -> Result<T, String> {
        Self::decode_record_in(bytes, wire, tag, EPOCH_WITHOUT_CATALOG)
    }
    fn encode_in(value: &T, _: Option<WireEncoding>, epoch: i64) -> Result<Vec<u8>, String> {
        value
            .encode(Some(epoch))
            .map(|r| r.bytes)
            .map_err(|r| format!("packed:{}: {r}", T::NAME))
    }
    fn decode_record_in(
        bytes: &[u8],
        _: Option<WireEncoding>,
        tag: Option<Tag>,
        epoch: i64,
    ) -> Result<T, String> {
        Self::record(tag, bytes, epoch)
    }
    fn carries(ty: Option<&TypeId>) -> bool {
        matches!(ty, Some(TypeId::Packed { name, .. }) if name == T::NAME)
    }
    fn tag() -> Option<Tag> {
        Tag::new(T::TAG)
    }
    fn json_reading(value: &T) -> Option<serde_json::Value> {
        Some(value.to_json())
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

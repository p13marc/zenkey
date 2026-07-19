//! Generic payload decode/encode against a [`TypeSchema`] (issue #11) —
//! feature `decode` (+ `decode-protobuf` for the protobuf kind).
//!
//! This lives in `zenkey`, not in any one tool, because every generic
//! consumer needs it: zenctl's echo, zengui's inspector, and `service call`'s
//! request building all go through the same [`DecoderRegistry`]. The decode
//! direction turns wire bytes into named-field JSON; the **encode** direction
//! is what powers schema-driven request forms (build a body from a JSON
//! value, framed for the target encoding).
//!
//! Zero-copy posture (report §14): decoders take `&[u8]` and the CBOR/JSON
//! paths parse in one pass; callers hand borrowed payload slices
//! (`ZBytes::slices()`/`reader()`), never `to_bytes().to_vec()` copies.

use serde_json::Value;

use super::{SchemaKind, TypeSchema, WireEncoding};

/// A decoded payload: named-field JSON plus honesty notes (fields the
/// schema does not declare, lossy conversions, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPayload {
    pub value: Value,
    /// Non-fatal observations a tool should surface (never silently drop).
    pub notes: Vec<String>,
}

/// A decode/encode failure.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("no decoder for schema kind {0:?} — render structurally instead")]
    UnknownKind(String),
    #[error("payload does not decode as {0}: {1}")]
    Malformed(&'static str, String),
    #[error("encoding {0:?} is not decodable under this schema kind")]
    WrongEncoding(String),
    #[error("schema entry is incomplete: {0}")]
    BadSchema(String),
    #[error("value does not conform for encoding: {0}")]
    Encode(String),
}

/// One schema kind's codec. Implementations are registered in a
/// [`DecoderRegistry`]; the registry dispatches on [`TypeSchema::kind`].
pub trait PayloadDecoder: Send + Sync {
    /// The schema kind this decoder serves (`json-schema`, `protobuf`, …).
    fn kind(&self) -> &str;

    /// Wire bytes → named-field JSON.
    fn decode(
        &self,
        schema: &TypeSchema,
        encoding: &WireEncoding,
        bytes: &[u8],
    ) -> Result<DecodedPayload, DecodeError>;

    /// JSON value → wire bytes in the target framing (the serialize half —
    /// schema-driven request forms).
    fn encode(
        &self,
        schema: &TypeSchema,
        value: &Value,
        target: &WireEncoding,
    ) -> Result<Vec<u8>, DecodeError>;
}

/// The `json-schema` codec: JSON and CBOR framings of the serde data model.
pub struct JsonSchemaDecoder;

impl JsonSchemaDecoder {
    /// Note top-level fields the schema does not declare (additive-evolution
    /// visibility — never an error, RFC 08 §3).
    fn undeclared_fields(schema: &TypeSchema, value: &Value) -> Vec<String> {
        let Some(doc) = schema.json_document() else {
            return Vec::new();
        };
        let Some(props) = doc.get("properties").and_then(Value::as_object) else {
            return Vec::new();
        };
        let Some(obj) = value.as_object() else {
            return Vec::new();
        };
        obj.keys()
            .filter(|k| !props.contains_key(*k))
            .map(|k| format!("field {k:?} is not in the served schema (additive evolution?)"))
            .collect()
    }
}

impl PayloadDecoder for JsonSchemaDecoder {
    fn kind(&self) -> &str {
        SchemaKind::JSON_SCHEMA
    }

    fn decode(
        &self,
        schema: &TypeSchema,
        encoding: &WireEncoding,
        bytes: &[u8],
    ) -> Result<DecodedPayload, DecodeError> {
        let value: Value = match encoding {
            WireEncoding::Json => serde_json::from_slice(bytes)
                .map_err(|e| DecodeError::Malformed("json", e.to_string()))?,
            WireEncoding::Cbor => {
                let cbor: ciborium::Value = ciborium::from_reader(bytes)
                    .map_err(|e| DecodeError::Malformed("cbor", e.to_string()))?;
                serde_json::to_value(&cbor)
                    .map_err(|e| DecodeError::Malformed("cbor->json", e.to_string()))?
            }
            other => return Err(DecodeError::WrongEncoding(format!("{other:?}"))),
        };
        let notes = Self::undeclared_fields(schema, &value);
        Ok(DecodedPayload { value, notes })
    }

    fn encode(
        &self,
        _schema: &TypeSchema,
        value: &Value,
        target: &WireEncoding,
    ) -> Result<Vec<u8>, DecodeError> {
        match target {
            WireEncoding::Json => {
                serde_json::to_vec(value).map_err(|e| DecodeError::Encode(e.to_string()))
            }
            WireEncoding::Cbor => {
                let mut out = Vec::new();
                ciborium::into_writer(value, &mut out)
                    .map_err(|e| DecodeError::Encode(e.to_string()))?;
                Ok(out)
            }
            other => Err(DecodeError::WrongEncoding(format!("{other:?}"))),
        }
    }
}

/// The `protobuf` codec (feature `decode-protobuf`): dynamic decode against
/// the served `FileDescriptorSet` — no compiled-in message types.
#[cfg(feature = "decode-protobuf")]
pub struct ProtobufDecoder;

#[cfg(feature = "decode-protobuf")]
impl ProtobufDecoder {
    fn descriptor(schema: &TypeSchema) -> Result<prost_reflect::MessageDescriptor, DecodeError> {
        let fds = schema
            .protobuf_descriptor_set()
            .ok_or_else(|| DecodeError::BadSchema("missing descriptor_b64".into()))?;
        let message = schema
            .protobuf_message()
            .ok_or_else(|| DecodeError::BadSchema("missing message name".into()))?;
        let pool = prost_reflect::DescriptorPool::decode(fds.as_slice())
            .map_err(|e| DecodeError::BadSchema(format!("descriptor set: {e}")))?;
        pool.get_message_by_name(message)
            .ok_or_else(|| DecodeError::BadSchema(format!("message {message:?} not in set")))
    }
}

#[cfg(feature = "decode-protobuf")]
impl PayloadDecoder for ProtobufDecoder {
    fn kind(&self) -> &str {
        SchemaKind::PROTOBUF
    }

    fn decode(
        &self,
        schema: &TypeSchema,
        encoding: &WireEncoding,
        bytes: &[u8],
    ) -> Result<DecodedPayload, DecodeError> {
        match encoding {
            WireEncoding::Protobuf | WireEncoding::Other(_) => {}
            WireEncoding::Json | WireEncoding::Cbor => {
                return Err(DecodeError::WrongEncoding(format!("{encoding:?}")));
            }
        }
        let desc = Self::descriptor(schema)?;
        let msg = prost_reflect::DynamicMessage::decode(desc, bytes)
            .map_err(|e| DecodeError::Malformed("protobuf", e.to_string()))?;
        let value = serde_json::to_value(&msg)
            .map_err(|e| DecodeError::Malformed("protobuf->json", e.to_string()))?;
        Ok(DecodedPayload {
            value,
            notes: Vec::new(),
        })
    }

    fn encode(
        &self,
        schema: &TypeSchema,
        value: &Value,
        _target: &WireEncoding,
    ) -> Result<Vec<u8>, DecodeError> {
        use prost::Message as _;
        let desc = Self::descriptor(schema)?;
        let rendered =
            serde_json::to_string(value).map_err(|e| DecodeError::Encode(e.to_string()))?;
        let mut deserializer = serde_json::Deserializer::from_str(&rendered);
        let msg = prost_reflect::DynamicMessage::deserialize(desc, &mut deserializer)
            .map_err(|e| DecodeError::Encode(e.to_string()))?;
        deserializer
            .end()
            .map_err(|e| DecodeError::Encode(e.to_string()))?;
        Ok(msg.encode_to_vec())
    }
}

/// Dispatch table: schema kind → codec. Built-ins are pre-registered;
/// applications may register custom kinds.
pub struct DecoderRegistry {
    decoders: Vec<Box<dyn PayloadDecoder>>,
}

impl Default for DecoderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DecoderRegistry {
    /// A registry with the built-in codecs (`json-schema`; `protobuf` when
    /// the `decode-protobuf` feature is on).
    pub fn new() -> Self {
        #[allow(unused_mut)]
        let mut decoders: Vec<Box<dyn PayloadDecoder>> = vec![Box::new(JsonSchemaDecoder)];
        #[cfg(feature = "decode-protobuf")]
        decoders.push(Box::new(ProtobufDecoder));
        DecoderRegistry { decoders }
    }

    /// Register a custom kind's codec (later registrations win on conflict).
    pub fn register(&mut self, decoder: Box<dyn PayloadDecoder>) {
        self.decoders.insert(0, decoder);
    }

    fn find(&self, kind: &str) -> Option<&dyn PayloadDecoder> {
        self.decoders
            .iter()
            .find(|d| d.kind() == kind)
            .map(Box::as_ref)
    }

    /// Decode wire bytes under a schema entry. `Err(UnknownKind)` means
    /// "render structurally and say so" — never a silent drop.
    pub fn decode(
        &self,
        schema: &TypeSchema,
        encoding: &WireEncoding,
        bytes: &[u8],
    ) -> Result<DecodedPayload, DecodeError> {
        self.find(schema.kind().as_str())
            .ok_or_else(|| DecodeError::UnknownKind(schema.kind().as_str().to_string()))?
            .decode(schema, encoding, bytes)
    }

    /// Encode a JSON value under a schema entry, framed for `target`.
    pub fn encode(
        &self,
        schema: &TypeSchema,
        value: &Value,
        target: &WireEncoding,
    ) -> Result<Vec<u8>, DecodeError> {
        self.find(schema.kind().as_str())
            .ok_or_else(|| DecodeError::UnknownKind(schema.kind().as_str().to_string()))?
            .encode(schema, value, target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn point_schema() -> TypeSchema {
        TypeSchema::json_schema(json!({
            "type": "object",
            "properties": { "x": { "type": "integer" }, "y": { "type": "integer" } },
        }))
    }

    #[test]
    fn json_and_cbor_framings_decode_to_the_same_value() {
        let registry = DecoderRegistry::new();
        let schema = point_schema();
        let value = json!({"x": 1, "y": 2});
        let json_bytes = serde_json::to_vec(&value).unwrap();
        let mut cbor_bytes = Vec::new();
        ciborium::into_writer(&value, &mut cbor_bytes).unwrap();
        let a = registry
            .decode(&schema, &WireEncoding::Json, &json_bytes)
            .unwrap();
        let b = registry
            .decode(&schema, &WireEncoding::Cbor, &cbor_bytes)
            .unwrap();
        assert_eq!(a.value, value);
        assert_eq!(b.value, value);
        assert!(a.notes.is_empty());
    }

    #[test]
    fn undeclared_fields_are_noted_not_fatal() {
        let registry = DecoderRegistry::new();
        let schema = point_schema();
        let bytes = serde_json::to_vec(&json!({"x": 1, "z": 9})).unwrap();
        let out = registry
            .decode(&schema, &WireEncoding::Json, &bytes)
            .unwrap();
        assert_eq!(out.notes.len(), 1);
        assert!(out.notes[0].contains("\"z\""));
    }

    #[test]
    fn encode_round_trips_both_framings() {
        let registry = DecoderRegistry::new();
        let schema = point_schema();
        let value = json!({"x": 7, "y": 8});
        for enc in [WireEncoding::Json, WireEncoding::Cbor] {
            let bytes = registry.encode(&schema, &value, &enc).unwrap();
            let back = registry.decode(&schema, &enc, &bytes).unwrap();
            assert_eq!(back.value, value, "{enc:?}");
        }
    }

    #[test]
    fn unknown_kind_is_an_honest_error() {
        let registry = DecoderRegistry::new();
        let json = r#"{
            "schema_version": 1, "app": "t",
            "types": { "W": { "kind": "cddl", "hash": "sha256:00", "spec": "x = int" } }
        }"#;
        let set = crate::schema::SchemaSet::parse(json).unwrap();
        let err = registry
            .decode(set.get("W").unwrap(), &WireEncoding::Json, b"{}")
            .unwrap_err();
        assert!(matches!(err, DecodeError::UnknownKind(k) if k == "cddl"));
    }

    /// Hand-encoded minimal FileDescriptorSet: package `t`, message `Blob`
    /// with `int32 x = 1` and `string name = 2`. Written against the
    /// descriptor.proto wire format so the fixture needs no protoc.
    #[cfg(feature = "decode-protobuf")]
    fn tiny_fds() -> Vec<u8> {
        fn ld(tag: u8, bytes: &[u8]) -> Vec<u8> {
            let mut out = vec![tag];
            out.push(u8::try_from(bytes.len()).unwrap());
            out.extend_from_slice(bytes);
            out
        }
        fn varint_field(tag: u8, v: u8) -> Vec<u8> {
            vec![tag, v]
        }
        // FieldDescriptorProto: name=1(str) number=3(varint) label=4 type=5 json_name=10(str)
        let field_x = {
            let mut f = ld(0x0a, b"x"); // name
            f.extend(varint_field(0x18, 1)); // number = 1
            f.extend(varint_field(0x20, 1)); // label = OPTIONAL
            f.extend(varint_field(0x28, 5)); // type = INT32
            f.extend(ld(0x52, b"x")); // json_name
            f
        };
        let field_name = {
            let mut f = ld(0x0a, b"name");
            f.extend(varint_field(0x18, 2));
            f.extend(varint_field(0x20, 1));
            f.extend(varint_field(0x28, 9)); // type = STRING
            f.extend(ld(0x52, b"name"));
            f
        };
        // DescriptorProto: name=1(str) field=2(repeated msg)
        let msg = {
            let mut m = ld(0x0a, b"Blob");
            m.extend(ld(0x12, &field_x));
            m.extend(ld(0x12, &field_name));
            m
        };
        // FileDescriptorProto: name=1 package=2 message_type=4
        let file = {
            let mut f = ld(0x0a, b"t.proto");
            f.extend(ld(0x12, b"t"));
            f.extend(ld(0x22, &msg));
            f
        };
        // FileDescriptorSet: file=1
        ld(0x0a, &file)
    }

    #[cfg(feature = "decode-protobuf")]
    #[test]
    fn protobuf_dynamic_decode_and_encode() {
        let registry = DecoderRegistry::new();
        let schema = TypeSchema::protobuf("t.Blob", &tiny_fds());
        // Encode from JSON via the dynamic message…
        let value = json!({"x": 42, "name": "hi"});
        let bytes = registry
            .encode(&schema, &value, &WireEncoding::Protobuf)
            .unwrap();
        // …and decode back to named fields without any compiled-in type.
        let out = registry
            .decode(&schema, &WireEncoding::Protobuf, &bytes)
            .unwrap();
        assert_eq!(out.value.get("x"), Some(&json!(42)));
        assert_eq!(out.value.get("name"), Some(&json!("hi")));
    }
}

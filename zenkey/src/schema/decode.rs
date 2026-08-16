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

#[cfg(any(feature = "decode-protobuf", feature = "validate-json"))]
use super::compiled::CompiledCache;
use super::validate::{NotValidated, Verdict};
use super::{SchemaKind, TypeSchema, WireEncoding};

/// A decoded payload: named-field JSON plus honesty notes (fields the
/// schema does not declare, lossy conversions, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPayload {
    pub value: Value,
    /// Non-fatal observations a tool should surface (never silently drop).
    pub notes: Vec<String>,
    /// Did the payload conform to its declared schema (#159)? Three states —
    /// a decode that was never validated must not render like one that
    /// passed. See [`super::validate`] for what "valid" claims per kind.
    pub verdict: Verdict,
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
///
/// With feature `validate-json` it also holds compiled draft 2020-12
/// validators, cached by schema hash exactly like the protobuf descriptor
/// pools (#100) — a schema compile must not sit on every echo line.
#[derive(Default)]
pub struct JsonSchemaDecoder {
    #[cfg(feature = "validate-json")]
    validators: CompiledCache<jsonschema::Validator>,
}

impl JsonSchemaDecoder {
    pub fn new() -> JsonSchemaDecoder {
        JsonSchemaDecoder::default()
    }

    /// The #159 verdict for a decoded value. Never a boolean: a schema that
    /// does not compile, or a build without the validator, is
    /// [`Verdict::NotValidated`] with its reason — not either answer.
    #[cfg(feature = "validate-json")]
    fn verdict(&self, schema: &TypeSchema, value: &Value) -> Verdict {
        let compiled = self.validators.get_or_compile(schema, |schema| {
            let doc = schema
                .json_document()
                .ok_or_else(|| DecodeError::BadSchema("missing json document".into()))?;
            jsonschema::validator_for(doc).map_err(|e| DecodeError::BadSchema(e.to_string()))
        });
        match compiled {
            Ok(validator) => super::validate::validate_json(&validator, value),
            Err(_) => Verdict::NotValidated(NotValidated::BadSchema),
        }
    }

    #[cfg(not(feature = "validate-json"))]
    fn verdict(&self, _schema: &TypeSchema, _value: &Value) -> Verdict {
        Verdict::NotValidated(NotValidated::FeatureOff)
    }

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
        let verdict = self.verdict(schema, &value);
        Ok(DecodedPayload {
            value,
            notes,
            verdict,
        })
    }

    fn encode(
        &self,
        schema: &TypeSchema,
        value: &Value,
        target: &WireEncoding,
    ) -> Result<Vec<u8>, DecodeError> {
        // #159: a value that violates the served schema is refused before it
        // is framed — this is what gives PrepareMode::Encode its teeth for
        // the json-schema kind. A schema that does not compile (or a build
        // without the validator) cannot judge the value and lets it pass;
        // refusing on our own failure would punish the payload for the tool.
        if let Verdict::Invalid(errors) = self.verdict(schema, value) {
            return Err(DecodeError::Encode(format!(
                "value violates the served schema: {}",
                errors.join("; ")
            )));
        }
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
///
/// Holds a [`CompiledCache`] of resolved message descriptors (issue #100):
/// building a `DescriptorPool` means base64-decoding the whole descriptor set
/// and parsing it, which was happening on every single sample in both
/// directions.
#[cfg(feature = "decode-protobuf")]
#[derive(Default)]
pub struct ProtobufDecoder {
    descriptors: CompiledCache<prost_reflect::MessageDescriptor>,
}

#[cfg(feature = "decode-protobuf")]
impl ProtobufDecoder {
    pub fn new() -> ProtobufDecoder {
        ProtobufDecoder::default()
    }

    /// How many descriptor pools this decoder has built — one per distinct
    /// schema hash (issue #100's acceptance is a counter, not an inspection).
    pub fn compilations(&self) -> u64 {
        self.descriptors.compilations()
    }

    fn descriptor(
        &self,
        schema: &TypeSchema,
    ) -> Result<std::sync::Arc<prost_reflect::MessageDescriptor>, DecodeError> {
        self.descriptors.get_or_compile(schema, |schema| {
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
        })
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
            // `Other` is an unlabelled bus, not a contradiction: the schema
            // says protobuf and the sample said nothing.
            WireEncoding::Protobuf | WireEncoding::Other(_) => {}
            WireEncoding::Json | WireEncoding::Cbor | WireEncoding::Cdr => {
                return Err(DecodeError::WrongEncoding(format!("{encoding:?}")));
            }
        }
        let desc = self.descriptor(schema)?;
        let msg = prost_reflect::DynamicMessage::decode((*desc).clone(), bytes)
            .map_err(|e| DecodeError::Malformed("protobuf", e.to_string()))?;
        let value = serde_json::to_value(&msg)
            .map_err(|e| DecodeError::Malformed("protobuf->json", e.to_string()))?;
        Ok(DecodedPayload {
            value,
            notes: Vec::new(),
            // A successful dynamic decode is structural conformance to the
            // served descriptor — the thinner Valid (see `validate`'s doc).
            verdict: Verdict::Valid,
        })
    }

    fn encode(
        &self,
        schema: &TypeSchema,
        value: &Value,
        _target: &WireEncoding,
    ) -> Result<Vec<u8>, DecodeError> {
        use prost::Message as _;
        let desc = self.descriptor(schema)?;
        let rendered =
            serde_json::to_string(value).map_err(|e| DecodeError::Encode(e.to_string()))?;
        let mut deserializer = serde_json::Deserializer::from_str(&rendered);
        let msg = prost_reflect::DynamicMessage::deserialize((*desc).clone(), &mut deserializer)
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
    /// the `decode-protobuf` feature is on; `cdr` when `decode-cdr` is).
    pub fn new() -> Self {
        #[allow(unused_mut)]
        let mut decoders: Vec<Box<dyn PayloadDecoder>> = vec![Box::new(JsonSchemaDecoder::new())];
        #[cfg(feature = "decode-protobuf")]
        decoders.push(Box::new(ProtobufDecoder::new()));
        #[cfg(feature = "decode-cdr")]
        decoders.push(Box::new(super::cdr::CdrDecoder::new()));
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

#[cfg(all(test, feature = "validate-json"))]
mod validate_tests {
    use super::*;
    use serde_json::json;

    fn strict_schema() -> TypeSchema {
        TypeSchema::json_schema(json!({
            "type": "object",
            "required": ["x"],
            "properties": { "x": { "type": "integer" } },
        }))
    }

    /// #159: the decode verdict is the schema's judgement, not the parser's.
    #[test]
    fn decode_verdicts_follow_the_schema() {
        let registry = DecoderRegistry::new();
        let s = strict_schema();
        let ok = serde_json::to_vec(&json!({"x": 1})).unwrap();
        let out = registry.decode(&s, &WireEncoding::Json, &ok).unwrap();
        assert_eq!(out.verdict, Verdict::Valid);

        let bad = serde_json::to_vec(&json!({"x": "seven"})).unwrap();
        let out = registry.decode(&s, &WireEncoding::Json, &bad).unwrap();
        match out.verdict {
            Verdict::Invalid(errors) => {
                assert_eq!(errors.len(), 1, "{errors:?}");
                assert!(errors[0].contains("/x"), "{errors:?}");
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    /// #159: PrepareMode::Encode's teeth — a nonconformant value is refused
    /// before framing, and the refusal names the violations.
    #[test]
    fn encode_refuses_a_nonconformant_value() {
        let registry = DecoderRegistry::new();
        let s = strict_schema();
        let err = registry
            .encode(&s, &json!({"x": "seven"}), &WireEncoding::Json)
            .unwrap_err();
        assert!(matches!(err, DecodeError::Encode(_)), "{err:?}");
        assert!(err.to_string().contains("violates"), "{err}");
        assert!(
            registry
                .encode(&s, &json!({"x": 1}), &WireEncoding::Json)
                .is_ok()
        );
    }

    /// The compiled validator is cached by schema hash, like the protobuf
    /// descriptor pools (#100).
    #[test]
    fn the_validator_compiles_once_across_many_samples() {
        let codec = JsonSchemaDecoder::new();
        let s = strict_schema();
        let bytes = serde_json::to_vec(&json!({"x": 1})).unwrap();
        for _ in 0..20 {
            codec.decode(&s, &WireEncoding::Json, &bytes).unwrap();
        }
        assert_eq!(codec.validators.compilations(), 1);
    }
}

#[cfg(all(test, feature = "decode-protobuf"))]
mod protobuf_compiled_tests {
    use super::*;

    /// `package t; message M { double v = 1; }`, optionally with a second
    /// field so two fixtures differ in hash.
    fn descriptor_set(extra_field: bool) -> Vec<u8> {
        use prost::Message as _;
        use prost_reflect::prost_types::{
            DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
            field_descriptor_proto,
        };
        let field = |name: &str, number: i32| FieldDescriptorProto {
            name: Some(name.to_string()),
            number: Some(number),
            label: Some(field_descriptor_proto::Label::Optional as i32),
            r#type: Some(field_descriptor_proto::Type::Double as i32),
            json_name: Some(name.to_string()),
            ..Default::default()
        };
        let mut fields = vec![field("v", 1)];
        if extra_field {
            fields.push(field("w", 2));
        }
        FileDescriptorSet {
            file: vec![FileDescriptorProto {
                name: Some("t.proto".into()),
                package: Some("t".into()),
                message_type: vec![DescriptorProto {
                    name: Some("M".into()),
                    field: fields,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
        .encode_to_vec()
    }

    fn schema(extra_field: bool) -> TypeSchema {
        TypeSchema::protobuf("t.M", &descriptor_set(extra_field))
    }

    /// Issue #100: the descriptor pool is built once per schema, not once per
    /// sample. Before this, `topic echo` on a protobuf subject paid a
    /// base64-decode plus a full pool parse for every message on the wire.
    #[test]
    fn the_descriptor_pool_is_built_once_across_many_samples() {
        let codec = ProtobufDecoder::new();
        let s = schema(false);
        let value = serde_json::json!({"v": 12.5});
        let bytes = codec
            .encode(&s, &value, &WireEncoding::Protobuf)
            .expect("encodes");

        for _ in 0..50 {
            let out = codec
                .decode(&s, &WireEncoding::Protobuf, &bytes)
                .expect("decodes");
            assert_eq!(out.value, value);
        }
        assert_eq!(
            codec.compilations(),
            1,
            "the descriptor set must be parsed once, not per sample"
        );
    }

    /// A producer that changes its type serves a new hash; the decoder must
    /// build the new pool rather than keep interpreting with the old one.
    #[test]
    fn a_changed_schema_hash_rebuilds_the_pool() {
        let codec = ProtobufDecoder::new();
        let (a, b) = (schema(false), schema(true));
        assert_ne!(a.hash(), b.hash(), "the fixture must actually differ");

        let av = serde_json::json!({"v": 1.0});
        let bv = serde_json::json!({"v": 1.0, "w": 2.0});
        let ab = codec.encode(&a, &av, &WireEncoding::Protobuf).unwrap();
        let bb = codec.encode(&b, &bv, &WireEncoding::Protobuf).unwrap();
        assert_eq!(codec.compilations(), 2);

        assert_eq!(
            codec
                .decode(&b, &WireEncoding::Protobuf, &bb)
                .unwrap()
                .value,
            bv,
            "the new schema decodes its own new field"
        );
        assert_eq!(
            codec
                .decode(&a, &WireEncoding::Protobuf, &ab)
                .unwrap()
                .value,
            av
        );
        assert_eq!(codec.compilations(), 2, "both pools were already built");
    }
}

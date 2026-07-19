//! Payload self-description (RFC 08 §7, issue #11) — feature `schema`.
//!
//! The registry binds one payload **type name** per subject (P5); this module
//! carries the **schema documents** for those names. A producer builds a
//! [`SchemaSet`] once and serves its JSON on `@rpc/<producer>/describe`; a
//! generic consumer (zenctl, zengui) fetches it on first miss, caches by
//! hash, and decodes any registered payload into named fields (`decode`
//! feature).
//!
//! Kinds are an **open vocabulary**: this crate registers `json-schema`
//! (describes the serde data model, so it covers both JSON and CBOR
//! framings) and `protobuf` (a base64 `FileDescriptorSet` + message name).
//! Parsing is tolerant exactly like the slice parser: unknown kinds are
//! retained-but-opaque, unknown fields ignored — one exotic type must not
//! blind a tool to the rest of the set.

#[cfg(feature = "decode")]
pub mod decode;

use std::collections::BTreeMap;
use std::fmt;

use serde_json::Value;
use sha2::{Digest, Sha256};

/// A schema-document kind. Open: compare against the registered constants,
/// pass unknown kinds through untouched.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SchemaKind(String);

impl SchemaKind {
    /// JSON Schema draft 2020-12 — covers JSON and CBOR framings of the
    /// same serde data model.
    pub const JSON_SCHEMA: &str = "json-schema";
    /// A base64 `FileDescriptorSet` plus a fully-qualified `message` name.
    pub const PROTOBUF: &str = "protobuf";

    pub fn new(kind: impl Into<String>) -> Self {
        SchemaKind(kind.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SchemaKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<str> for SchemaKind {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for SchemaKind {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// The wire framing of a payload — a separate axis from its schema
/// (RFC 08 §7): one `json-schema` document describes both the JSON and the
/// CBOR framing of a type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireEncoding {
    Json,
    Cbor,
    Protobuf,
    /// Anything else — carried verbatim, decoded only by sniff.
    Other(String),
}

impl WireEncoding {
    /// Map a middleware/registry encoding string (`application/cbor`, …).
    pub fn from_encoding_str(s: &str) -> WireEncoding {
        match s {
            "application/json" | "text/json" => WireEncoding::Json,
            "application/cbor" => WireEncoding::Cbor,
            "application/protobuf" | "application/x-protobuf" => WireEncoding::Protobuf,
            other => WireEncoding::Other(other.to_string()),
        }
    }
}

/// One type's schema entry in a [`SchemaSet`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeSchema {
    kind: SchemaKind,
    /// `sha256:<hex>` over the schema's canonical bytes.
    hash: String,
    /// The full entry body (kind-specific fields included), minus `kind`
    /// and `hash`. For `json-schema`: `{"schema": {...}}`. For `protobuf`:
    /// `{"message": "...", "descriptor_b64": "..."}`. Unknown kinds keep
    /// whatever they carried.
    body: BTreeMap<String, Value>,
}

/// Canonical-bytes hash (RFC 08 §7): serde_json compact serialization with
/// lexicographically ordered keys (this crate's `Value` maps are BTree-backed,
/// so `to_vec` is already ordered) — JCS-compatible for the value domain
/// schema documents actually use.
fn hash_value(v: &Value) -> String {
    let bytes = serde_json::to_vec(v).expect("Value serializes");
    let digest = Sha256::digest(&bytes);
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

impl TypeSchema {
    /// A `json-schema` entry from an explicit schema document.
    pub fn json_schema(document: Value) -> TypeSchema {
        let hash = hash_value(&document);
        let mut body = BTreeMap::new();
        body.insert("schema".to_string(), document);
        TypeSchema {
            kind: SchemaKind::new(SchemaKind::JSON_SCHEMA),
            hash,
            body,
        }
    }

    /// A `json-schema` entry derived from a serde type (feature `schemars`;
    /// honors `#[serde(...)]` attributes).
    #[cfg(feature = "schemars")]
    pub fn json_schema_of<T: schemars::JsonSchema>() -> TypeSchema {
        let schema = schemars::schema_for!(T);
        let document = serde_json::to_value(schema).expect("schema serializes");
        Self::json_schema(document)
    }

    /// A `protobuf` entry: the fully-qualified message name and the raw
    /// `FileDescriptorSet` bytes (from `prost-build`'s
    /// `file_descriptor_set_path`). Serving needs no protobuf dependency —
    /// the bytes are carried base64.
    pub fn protobuf(message: impl Into<String>, descriptor_set: &[u8]) -> TypeSchema {
        use base64::Engine as _;
        let digest = Sha256::digest(descriptor_set);
        let mut hash = String::with_capacity(7 + 64);
        hash.push_str("sha256:");
        for b in digest {
            use std::fmt::Write as _;
            let _ = write!(hash, "{b:02x}");
        }
        let mut body = BTreeMap::new();
        body.insert("message".to_string(), Value::String(message.into()));
        body.insert(
            "descriptor_b64".to_string(),
            Value::String(base64::engine::general_purpose::STANDARD.encode(descriptor_set)),
        );
        TypeSchema {
            kind: SchemaKind::new(SchemaKind::PROTOBUF),
            hash,
            body,
        }
    }

    pub fn kind(&self) -> &SchemaKind {
        &self.kind
    }

    /// The `sha256:` cache/drift key.
    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// The JSON Schema document, when this is a `json-schema` entry.
    pub fn json_document(&self) -> Option<&Value> {
        (self.kind == SchemaKind::JSON_SCHEMA)
            .then(|| self.body.get("schema"))
            .flatten()
    }

    /// The protobuf message name, when this is a `protobuf` entry.
    pub fn protobuf_message(&self) -> Option<&str> {
        (self.kind == SchemaKind::PROTOBUF)
            .then(|| self.body.get("message").and_then(Value::as_str))
            .flatten()
    }

    /// The decoded `FileDescriptorSet` bytes, when this is a `protobuf` entry.
    pub fn protobuf_descriptor_set(&self) -> Option<Vec<u8>> {
        use base64::Engine as _;
        (self.kind == SchemaKind::PROTOBUF)
            .then(|| {
                self.body
                    .get("descriptor_b64")
                    .and_then(Value::as_str)
                    .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
            })
            .flatten()
    }
}

/// A schema-set parse/build failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchemaError {
    #[error("schema set does not parse: {0}")]
    Parse(String),
    #[error("unsupported schema_version {0} (this reader knows 1)")]
    Version(i64),
    #[error("schema set does not cover registry type(s): {}", .0.join(", "))]
    Coverage(Vec<String>),
}

/// A producer's full type-schema inventory — the `describe` reply
/// (RFC 08 §7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaSet {
    app: String,
    types: BTreeMap<String, TypeSchema>,
}

impl SchemaSet {
    /// Start building a set (producer side).
    pub fn builder(app: impl Into<String>) -> SchemaSetBuilder {
        SchemaSetBuilder {
            set: SchemaSet {
                app: app.into(),
                types: BTreeMap::new(),
            },
        }
    }

    /// The owning application, as declared.
    pub fn app(&self) -> &str {
        &self.app
    }

    pub fn get(&self, type_name: &str) -> Option<&TypeSchema> {
        self.types.get(type_name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &TypeSchema)> {
        self.types.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn len(&self) -> usize {
        self.types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// The RFC 08 §7 totality bound: every registry-referenced type name
    /// must be present (a superset is fine). Feed it the generated
    /// `TYPE_NAMES` const — this replaces the hand-written
    /// `types_are_total` CI tests adopters kept.
    pub fn verify_covers(&self, names: &[&str]) -> Result<(), SchemaError> {
        let missing: Vec<String> = names
            .iter()
            .filter(|n| !self.types.contains_key(**n))
            .map(|n| n.to_string())
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(SchemaError::Coverage(missing))
        }
    }

    /// Serialize the `describe` reply.
    pub fn to_json(&self) -> String {
        let mut types = serde_json::Map::new();
        for (name, t) in &self.types {
            let mut entry = serde_json::Map::new();
            entry.insert(
                "kind".to_string(),
                Value::String(t.kind.as_str().to_string()),
            );
            entry.insert("hash".to_string(), Value::String(t.hash.clone()));
            for (k, v) in &t.body {
                entry.insert(k.clone(), v.clone());
            }
            types.insert(name.clone(), Value::Object(entry));
        }
        let doc = serde_json::json!({
            "schema_version": 1,
            "app": self.app,
            "types": types,
        });
        serde_json::to_string(&doc).expect("schema set serializes")
    }

    /// Parse a served `describe` reply. Tolerant (RFC 08 §7): unknown
    /// fields are ignored, unknown kinds are retained as opaque entries —
    /// intolerant only of a missing/malformed required shape.
    pub fn parse(json: &str) -> Result<SchemaSet, SchemaError> {
        let doc: Value =
            serde_json::from_str(json).map_err(|e| SchemaError::Parse(e.to_string()))?;
        let version = doc
            .get("schema_version")
            .and_then(Value::as_i64)
            .ok_or_else(|| SchemaError::Parse("missing schema_version".into()))?;
        if version != 1 {
            return Err(SchemaError::Version(version));
        }
        let app = doc
            .get("app")
            .and_then(Value::as_str)
            .ok_or_else(|| SchemaError::Parse("missing app".into()))?
            .to_string();
        let raw_types = doc
            .get("types")
            .and_then(Value::as_object)
            .ok_or_else(|| SchemaError::Parse("missing types".into()))?;
        let mut types = BTreeMap::new();
        for (name, entry) in raw_types {
            let Some(obj) = entry.as_object() else {
                return Err(SchemaError::Parse(format!(
                    "type {name:?} is not an object"
                )));
            };
            let kind = obj
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| SchemaError::Parse(format!("type {name:?} missing kind")))?;
            let hash = obj
                .get("hash")
                .and_then(Value::as_str)
                .ok_or_else(|| SchemaError::Parse(format!("type {name:?} missing hash")))?;
            let body: BTreeMap<String, Value> = obj
                .iter()
                .filter(|(k, _)| k.as_str() != "kind" && k.as_str() != "hash")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            types.insert(
                name.clone(),
                TypeSchema {
                    kind: SchemaKind::new(kind),
                    hash: hash.to_string(),
                    body,
                },
            );
        }
        Ok(SchemaSet { app, types })
    }
}

/// Builder for a producer's [`SchemaSet`].
pub struct SchemaSetBuilder {
    set: SchemaSet,
}

impl SchemaSetBuilder {
    /// Register a serde type under its registry name (feature `schemars`).
    #[cfg(feature = "schemars")]
    pub fn json<T: schemars::JsonSchema>(mut self, name: impl Into<String>) -> Self {
        self.set
            .types
            .insert(name.into(), TypeSchema::json_schema_of::<T>());
        self
    }

    /// Register an explicit schema entry.
    pub fn entry(mut self, name: impl Into<String>, schema: TypeSchema) -> Self {
        self.set.types.insert(name.into(), schema);
        self
    }

    pub fn build(self) -> SchemaSet {
        self.set
    }

    /// Build and enforce the totality bound against the generated
    /// `TYPE_NAMES` (RFC 08 §7).
    ///
    /// # Panics
    /// On a coverage gap — this is the producer-side CI check, meant for a
    /// `static`/startup path where a gap must be loud.
    pub fn build_verified(self, names: &[&str]) -> SchemaSet {
        if let Err(e) = self.set.verify_covers(names) {
            panic!("{e}");
        }
        self.set
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_set() -> SchemaSet {
        SchemaSet::builder("t")
            .entry(
                "Point",
                TypeSchema::json_schema(serde_json::json!({
                    "type": "object",
                    "properties": { "x": { "type": "integer" } },
                })),
            )
            .entry("Blob", TypeSchema::protobuf("t.Blob", b"\x0a\x00"))
            .build()
    }

    #[test]
    fn round_trips_through_json() {
        let set = sample_set();
        let parsed = SchemaSet::parse(&set.to_json()).unwrap();
        assert_eq!(parsed, set);
        assert_eq!(parsed.app(), "t");
        assert!(parsed.get("Point").unwrap().json_document().is_some());
        assert_eq!(
            parsed.get("Blob").unwrap().protobuf_message(),
            Some("t.Blob")
        );
        assert_eq!(
            parsed
                .get("Blob")
                .unwrap()
                .protobuf_descriptor_set()
                .unwrap(),
            b"\x0a\x00"
        );
    }

    #[test]
    fn hash_is_stable_and_prefixed() {
        let a = TypeSchema::json_schema(serde_json::json!({"b": 1, "a": 2}));
        let b = TypeSchema::json_schema(serde_json::json!({"a": 2, "b": 1}));
        // Key order does not matter: BTree-backed maps serialize sorted.
        assert_eq!(a.hash(), b.hash());
        assert!(a.hash().starts_with("sha256:"));
        let c = TypeSchema::json_schema(serde_json::json!({"a": 3}));
        assert_ne!(a.hash(), c.hash());
    }

    #[test]
    fn unknown_kinds_are_retained_not_fatal() {
        let json = r#"{
            "schema_version": 1,
            "app": "foreign",
            "types": {
                "Weird": { "kind": "cddl", "hash": "sha256:00", "spec": "x = int", "extra": 1 },
                "Point": { "kind": "json-schema", "hash": "sha256:11", "schema": {} }
            }
        }"#;
        let set = SchemaSet::parse(json).unwrap();
        assert_eq!(set.len(), 2);
        assert_eq!(set.get("Weird").unwrap().kind(), &"cddl");
        // Opaque but present: a tool can still report it honestly.
        assert!(set.get("Weird").unwrap().json_document().is_none());
    }

    #[test]
    fn verify_covers_reports_gaps() {
        let set = sample_set();
        assert!(set.verify_covers(&["Point", "Blob"]).is_ok());
        let err = set.verify_covers(&["Point", "Missing"]).unwrap_err();
        assert!(matches!(err, SchemaError::Coverage(ref v) if v == &["Missing"]));
    }

    #[test]
    fn future_versions_are_refused_loudly() {
        let json = r#"{ "schema_version": 2, "app": "x", "types": {} }"#;
        assert!(matches!(
            SchemaSet::parse(json),
            Err(SchemaError::Version(2))
        ));
    }

    #[cfg(feature = "schemars")]
    #[test]
    fn schemars_derivation() {
        #[derive(schemars::JsonSchema)]
        #[allow(dead_code)]
        struct Health {
            status: String,
            uptime_s: u64,
        }
        let set = SchemaSet::builder("t")
            .json::<Health>("Health")
            .build_verified(&["Health"]);
        let doc = set.get("Health").unwrap().json_document().unwrap();
        let props = doc.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("status"));
        assert!(props.contains_key("uptime_s"));
    }

    #[test]
    fn wire_encoding_mapping() {
        assert_eq!(
            WireEncoding::from_encoding_str("application/cbor"),
            WireEncoding::Cbor
        );
        assert_eq!(
            WireEncoding::from_encoding_str("application/json"),
            WireEncoding::Json
        );
        assert_eq!(
            WireEncoding::from_encoding_str("video/h264"),
            WireEncoding::Other("video/h264".into())
        );
    }
}

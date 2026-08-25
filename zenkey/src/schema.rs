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
//! framings), `protobuf` (a base64 `FileDescriptorSet` + message name), and
//! `cdr` (a compact field list for the DDS / ROS 2 framing, v1.10).
//! Parsing is tolerant exactly like the slice parser: unknown kinds are
//! retained-but-opaque, unknown fields ignored — one exotic type must not
//! blind a tool to the rest of the set.

#[cfg(feature = "decode-cdr")]
pub mod cdr;
#[cfg(feature = "decode")]
pub mod compiled;
#[cfg(feature = "decode")]
pub mod decode;
#[cfg(feature = "decode")]
pub mod validate;

use std::collections::BTreeMap;

pub use crate::encoding::WireEncoding;
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
    /// A compact JSON field list describing an XCDR1 message (DDS / ROS 2),
    /// with the `.msg`/IDL source text carried informatively (v1.10).
    pub const CDR: &str = "cdr";

    #[must_use]
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

/// One type's schema entry in a [`SchemaSet`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeSchema {
    /// `sha256:<hex>` over the schema's canonical bytes.
    ///
    /// `None` where the served document carried no identity — an absent or
    /// empty `hash`. RFC 08 §7 says the hash "exists for client caching", and
    /// an empty string is not an identity to cache under: two unrelated types
    /// would share it. Spelling that as `None` is why
    /// [`CompiledCache`](crate::schema::compiled::CompiledCache) no longer
    /// compares against `""` (#323).
    hash: Option<String>,
    /// What kind of schema this is, and its kind-specific fields.
    body: SchemaBody,
}

/// A schema entry's kind and the fields that kind carries (RFC 08 §7).
///
/// This is the tagged union the wire document always was: `kind` names the
/// variant and the remaining keys are its fields. Carried as a
/// `SchemaKind` + `BTreeMap<String, Value>` it was a union only by
/// convention, so a `protobuf` entry with a `schema` key and no `message`
/// was constructible, and every reader gated on `kind` by hand before
/// reaching into the map (#323).
///
/// `Other` keeps RFC 08 §7's tolerance: a kind this build does not know is
/// carried verbatim and re-serialized unchanged, never rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaBody {
    /// `json-schema` — one JSON Schema document under `schema`.
    JsonSchema { schema: Value },
    /// `protobuf` — a message name and a base64 `FileDescriptorSet`.
    Protobuf {
        message: String,
        descriptor_b64: String,
    },
    /// `cdr` — an ordered field list, an optional local type table, and an
    /// optional informative `source` (v1.10).
    Cdr {
        fields: Value,
        types: Option<Value>,
        /// Informative only: deliberately outside the hash, because two
        /// producers generating the same message from `.msg` and from IDL
        /// describe the same wire format.
        source: Option<Value>,
    },
    /// A kind this build does not know, carried verbatim.
    Other {
        kind: SchemaKind,
        fields: BTreeMap<String, Value>,
    },
}

impl SchemaBody {
    /// The `kind` token this body serializes under.
    pub fn kind(&self) -> SchemaKind {
        match self {
            SchemaBody::JsonSchema { .. } => SchemaKind::new(SchemaKind::JSON_SCHEMA),
            SchemaBody::Protobuf { .. } => SchemaKind::new(SchemaKind::PROTOBUF),
            SchemaBody::Cdr { .. } => SchemaKind::new(SchemaKind::CDR),
            SchemaBody::Other { kind, .. } => kind.clone(),
        }
    }

    /// The `kind` token, borrowed — for the common case of writing it into a
    /// string or a report field without building a [`SchemaKind`].
    pub fn kind_str(&self) -> &str {
        match self {
            SchemaBody::JsonSchema { .. } => SchemaKind::JSON_SCHEMA,
            SchemaBody::Protobuf { .. } => SchemaKind::PROTOBUF,
            SchemaBody::Cdr { .. } => SchemaKind::CDR,
            SchemaBody::Other { kind, .. } => kind.as_str(),
        }
    }

    /// The entry's fields, as the wire spells them — everything except
    /// `kind` and `hash`.
    pub fn fields(&self) -> BTreeMap<String, Value> {
        let mut out = BTreeMap::new();
        match self {
            SchemaBody::JsonSchema { schema } => {
                out.insert("schema".to_string(), schema.clone());
            }
            SchemaBody::Protobuf {
                message,
                descriptor_b64,
            } => {
                out.insert("message".to_string(), Value::String(message.clone()));
                out.insert(
                    "descriptor_b64".to_string(),
                    Value::String(descriptor_b64.clone()),
                );
            }
            SchemaBody::Cdr {
                fields,
                types,
                source,
            } => {
                out.insert("fields".to_string(), fields.clone());
                if let Some(t) = types {
                    out.insert("types".to_string(), t.clone());
                }
                if let Some(src) = source {
                    out.insert("source".to_string(), src.clone());
                }
            }
            SchemaBody::Other { fields, .. } => out = fields.clone(),
        }
        out
    }

    /// Read a served entry: a known kind into its variant, anything else
    /// carried whole.
    fn parse(kind: &str, fields: BTreeMap<String, Value>) -> SchemaBody {
        let take = |k: &str| fields.get(k).cloned();
        match kind {
            SchemaKind::JSON_SCHEMA => match take("schema") {
                Some(schema) => SchemaBody::JsonSchema { schema },
                // A `json-schema` entry with no document is not a
                // `json-schema` entry this build can use — carried, not
                // silently treated as an empty schema that validates
                // everything.
                None => SchemaBody::Other {
                    kind: SchemaKind::new(kind),
                    fields,
                },
            },
            SchemaKind::PROTOBUF => {
                match (
                    take("message").and_then(|v| v.as_str().map(str::to_string)),
                    take("descriptor_b64").and_then(|v| v.as_str().map(str::to_string)),
                ) {
                    (Some(message), Some(descriptor_b64)) => SchemaBody::Protobuf {
                        message,
                        descriptor_b64,
                    },
                    _ => SchemaBody::Other {
                        kind: SchemaKind::new(kind),
                        fields,
                    },
                }
            }
            SchemaKind::CDR => match take("fields") {
                Some(f) => SchemaBody::Cdr {
                    fields: f,
                    types: take("types"),
                    source: take("source"),
                },
                None => SchemaBody::Other {
                    kind: SchemaKind::new(kind),
                    fields,
                },
            },
            _ => SchemaBody::Other {
                kind: SchemaKind::new(kind),
                fields,
            },
        }
    }
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
    #[must_use]
    pub fn json_schema(document: Value) -> TypeSchema {
        let hash = hash_value(&document);
        TypeSchema {
            hash: Some(hash),
            body: SchemaBody::JsonSchema { schema: document },
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
    #[must_use]
    pub fn protobuf(message: impl Into<String>, descriptor_set: &[u8]) -> TypeSchema {
        use base64::Engine as _;
        let digest = Sha256::digest(descriptor_set);
        let mut hash = String::with_capacity(7 + 64);
        hash.push_str("sha256:");
        for b in digest {
            use std::fmt::Write as _;
            let _ = write!(hash, "{b:02x}");
        }
        TypeSchema {
            hash: Some(hash),
            body: SchemaBody::Protobuf {
                message: message.into(),
                descriptor_b64: base64::engine::general_purpose::STANDARD.encode(descriptor_set),
            },
        }
    }

    /// A `cdr` entry (v1.10) from its document: `fields` (required), `types`
    /// (optional local type table), and `source` (optional, informative).
    ///
    /// The hash covers **`fields` and `types` only**. The source text is the
    /// human's copy of the schema, not the schema: two producers generating
    /// the same message from `.msg` and from IDL describe the same wire
    /// format, and drift detection must not call that a disagreement.
    #[must_use]
    pub fn cdr(document: Value) -> TypeSchema {
        let mut hashed = serde_json::Map::new();
        for key in ["fields", "types"] {
            if let Some(v) = document.get(key) {
                hashed.insert(key.to_string(), v.clone());
            }
        }
        let hash = hash_value(&Value::Object(hashed));
        TypeSchema {
            hash: Some(hash),
            body: SchemaBody::Cdr {
                // `fields` is the one required key; a document without it is
                // not a `cdr` entry, and `Value::Null` is what it carried.
                fields: document.get("fields").cloned().unwrap_or(Value::Null),
                types: document.get("types").cloned(),
                source: document.get("source").cloned(),
            },
        }
    }

    /// The `kind` token this entry serializes under.
    pub fn kind(&self) -> SchemaKind {
        self.body.kind()
    }

    /// The `kind` token, borrowed.
    pub fn kind_str(&self) -> &str {
        self.body.kind_str()
    }

    /// The entry's kind-specific fields, typed.
    pub fn body(&self) -> &SchemaBody {
        &self.body
    }

    /// The `sha256:` cache/drift key, when the served document carried one.
    ///
    /// `None` is *no identity*, which is why it is not a `&str`: an empty
    /// string used to mean this, and every caller had to remember to check
    /// for it (#323).
    pub fn hash(&self) -> Option<&str> {
        self.hash.as_deref()
    }

    /// The JSON Schema document, when this is a `json-schema` entry.
    pub fn json_document(&self) -> Option<&Value> {
        match &self.body {
            SchemaBody::JsonSchema { schema } => Some(schema),
            _ => None,
        }
    }

    /// The protobuf message name, when this is a `protobuf` entry.
    pub fn protobuf_message(&self) -> Option<&str> {
        match &self.body {
            SchemaBody::Protobuf { message, .. } => Some(message),
            _ => None,
        }
    }

    /// The ordered field list, when this is a `cdr` entry.
    pub fn cdr_fields(&self) -> Option<&Value> {
        match &self.body {
            SchemaBody::Cdr { fields, .. } => Some(fields),
            _ => None,
        }
    }

    /// The local type table of a `cdr` entry (absent when the message uses
    /// primitives only).
    pub fn cdr_types(&self) -> Option<&serde_json::Map<String, Value>> {
        match &self.body {
            SchemaBody::Cdr { types, .. } => types.as_ref().and_then(Value::as_object),
            _ => None,
        }
    }

    /// The decoded `FileDescriptorSet` bytes, when this is a `protobuf` entry.
    pub fn protobuf_descriptor_set(&self) -> Option<Vec<u8>> {
        use base64::Engine as _;
        match &self.body {
            SchemaBody::Protobuf { descriptor_b64, .. } => {
                base64::engine::general_purpose::STANDARD
                    .decode(descriptor_b64)
                    .ok()
            }
            _ => None,
        }
    }
}

/// A schema-set parse/build failure.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    /// The reply is not well-formed JSON. The `serde_json` error is the
    /// source, so a caller reaches its line and column (#317).
    #[error("schema set does not parse: {0}")]
    Json(#[from] serde_json::Error),
    /// It parses as JSON but is not a schema set. No underlying error: this
    /// crate is the one saying so.
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
    #[must_use]
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
            entry.insert("kind".to_string(), Value::String(t.kind_str().to_string()));
            // The wire field is a string and stays one: `None` re-serializes
            // as `""`, which is what a document with no identity carried in.
            entry.insert(
                "hash".to_string(),
                Value::String(t.hash.clone().unwrap_or_default()),
            );
            for (k, v) in t.body.fields() {
                entry.insert(k, v);
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
        let doc: Value = serde_json::from_str(json)?;
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
                    hash: (!hash.is_empty()).then(|| hash.to_string()),
                    body: SchemaBody::parse(kind, body),
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
    #[must_use]
    pub fn entry(mut self, name: impl Into<String>, schema: TypeSchema) -> Self {
        self.set.types.insert(name.into(), schema);
        self
    }

    #[must_use]
    pub fn build(self) -> SchemaSet {
        self.set
    }

    /// Build and enforce the totality bound against the generated
    /// `TYPE_NAMES` (RFC 08 §7).
    ///
    /// # Panics
    /// On a coverage gap — this is the producer-side CI check, meant for a
    /// `static`/startup path where a gap must be loud.
    #[must_use]
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
        assert!(a.hash().is_some_and(|h| h.starts_with("sha256:")));
        let c = TypeSchema::json_schema(serde_json::json!({"a": 3}));
        assert_ne!(a.hash(), c.hash());
    }

    /// RFC 08 §7's own forward-compat rule, applied to the kind this
    /// amendment adds: a consumer built before `cdr` existed (or with the
    /// feature off) must skip it and keep the rest of the set. The `cdr`
    /// entry is *retained*, not dropped — a tool can still name the type it
    /// cannot decode, which is what makes the gap reportable.
    #[test]
    fn an_older_consumer_skips_cdr_without_losing_the_set() {
        let json = r#"{
            "schema_version": 1,
            "app": "ros",
            "types": {
                "Twist": { "kind": "cdr", "hash": "sha256:aa",
                           "fields": [{"name": "x", "type": "float64"}] },
                "Health": { "kind": "json-schema", "hash": "sha256:bb", "schema": {} }
            }
        }"#;
        let set = SchemaSet::parse(json).unwrap();
        assert_eq!(set.len(), 2, "the unknown kind must not blind us to Health");
        assert!(set.get("Health").unwrap().json_document().is_some());
        assert_eq!(set.get("Twist").unwrap().kind(), SchemaKind::CDR);
        // Round-tripping preserves the entry verbatim, feature or no feature.
        assert_eq!(SchemaSet::parse(&set.to_json()).unwrap(), set);
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
        assert_eq!(set.get("Weird").unwrap().kind(), "cddl");
        // Opaque but present: a tool can still report it honestly.
        assert!(set.get("Weird").unwrap().json_document().is_none());
        // …and it is `Other`, carrying every field it arrived with, so the
        // re-serialization below is byte-identical in content (#323).
        assert!(matches!(
            set.get("Weird").unwrap().body(),
            SchemaBody::Other { .. }
        ));
        let back = SchemaSet::parse(&set.to_json()).unwrap();
        assert_eq!(back, set);
    }

    /// A served entry whose kind is known but whose required field is absent
    /// is **not** that kind. It degrades to `Other` and is carried, rather
    /// than becoming a `json-schema` with no document — which would validate
    /// everything — or a `protobuf` with no message (#323).
    #[test]
    fn a_known_kind_missing_its_required_field_degrades_rather_than_lying() {
        let json = r#"{
            "schema_version": 1,
            "app": "foreign",
            "types": {
                "NoDoc":  { "kind": "json-schema", "hash": "sha256:00" },
                "NoMsg":  { "kind": "protobuf", "hash": "sha256:11", "descriptor_b64": "AA==" },
                "NoField":{ "kind": "cdr", "hash": "sha256:22", "source": "x" }
            }
        }"#;
        let set = SchemaSet::parse(json).unwrap();
        for name in ["NoDoc", "NoMsg", "NoField"] {
            let t = set.get(name).unwrap();
            assert!(
                matches!(t.body(), SchemaBody::Other { .. }),
                "{name} should degrade to Other"
            );
            assert!(t.json_document().is_none());
            assert!(t.protobuf_message().is_none());
            assert!(t.cdr_fields().is_none());
        }
        // The kind token is still reported as served — degrading the *body*
        // must not rewrite what the producer said it was.
        assert_eq!(set.get("NoDoc").unwrap().kind_str(), "json-schema");
    }

    /// The hash is an identity, and "no identity" is `None`, not `""` — the
    /// sentinel every caller used to have to remember (#323).
    #[test]
    fn an_absent_hash_is_none_and_re_serializes_as_it_arrived() {
        let json = r#"{
            "schema_version": 1,
            "app": "foreign",
            "types": { "Point": { "kind": "json-schema", "hash": "", "schema": {} } }
        }"#;
        let set = SchemaSet::parse(json).unwrap();
        assert_eq!(set.get("Point").unwrap().hash(), None);
        // The wire field is a string, so it goes back out as it came in.
        assert!(set.to_json().contains(r#""hash":"""#));
        assert_eq!(SchemaSet::parse(&set.to_json()).unwrap(), set);

        // A schema this crate builds always has one.
        assert!(
            TypeSchema::json_schema(serde_json::json!({}))
                .hash()
                .is_some()
        );
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
            WireEncoding::from_encoding_str("application/cdr"),
            WireEncoding::Cdr
        );
        assert_eq!(
            WireEncoding::from_encoding_str("video/h264"),
            WireEncoding::Other("video/h264".into())
        );
    }
}

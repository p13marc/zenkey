//! Bundles: the canonical contract with every schema artifact it lists
//! (r3 §3.11, the r3.1 container).
//!
//! ```json
//! {"contract": <canonical contract>,
//!  "schemas": {"sha256:…": {"kind": "protobuf", "data": "<base64 FileDescriptorSet>"},
//!              "sha256:…": {"kind": "jsonschema", "data": <JSON Schema document>}},
//!  "extras": {}}
//! ```
//!
//! Integrity is Merkle-style: the fingerprint is computed over the
//! canonical contract, and the contract lists every schema's id, so
//! verifying each schema against its id verifies the whole bundle. A bundle
//! file is the JCS serialization of that object, so one contract revision
//! always produces the same bytes. `extras` (`views.v1` documents, D18) are
//! carried and hashed, never interpreted.

use std::collections::BTreeMap;

use base64::Engine as _;
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::canonical::{FORMAT, Fingerprint, canonical, check_restrictions};
use crate::contract::Contract;
use crate::diag::Report;
use crate::schema::{jcs, sha256_id};

/// Why a bundle was refused.
#[derive(Debug, Error)]
pub enum BundleError {
    #[error("not strict JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("bundle shape: {0}")]
    Shape(String),
    #[error("the contract's format is {0:?}, not {FORMAT:?}")]
    Format(String),
    #[error("the canonical contract breaks the restrictions: {0}")]
    Restrictions(String),
    #[error("schema {0} is listed by the contract but missing from the bundle")]
    MissingSchema(String),
    #[error("schema {0} is in the bundle but not listed by the contract")]
    UnlistedSchema(String),
    #[error("schema {id}: kind {got:?} where the contract lists {want:?}")]
    SchemaKind {
        id: String,
        got: String,
        want: String,
    },
    #[error("schema {id}: its content hashes to {got}")]
    SchemaHash { id: String, got: String },
    #[error("extra {id}: its content hashes to {got}")]
    ExtraHash { id: String, got: String },
    #[error("the bundle's fingerprint is {got}, not the expected {want}")]
    Fingerprint { got: Fingerprint, want: Fingerprint },
}

impl BundleError {
    /// A stable, implementation-neutral name for the refusal, as the
    /// conformance fixtures spell it (`spec/conformance/bundles/`).
    #[must_use]
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Json(_) => "json",
            Self::Shape(_) => "shape",
            Self::Format(_) => "format",
            Self::Restrictions(_) => "restrictions",
            Self::MissingSchema(_) => "missing_schema",
            Self::UnlistedSchema(_) => "unlisted_schema",
            Self::SchemaKind { .. } => "schema_kind",
            Self::SchemaHash { .. } => "schema_hash",
            Self::ExtraHash { .. } => "extra_hash",
            Self::Fingerprint { .. } => "fingerprint",
        }
    }
}

/// A bundle, in memory.
#[derive(Debug, Clone, PartialEq)]
pub struct Bundle {
    /// The canonical contract.
    pub contract: Value,
    /// Schema artifacts by id: `{kind, data}`.
    pub schemas: BTreeMap<String, Value>,
    /// Extra artifacts by id (`views.v1` documents): `{media_type, data}`.
    pub extras: BTreeMap<String, Value>,
}

impl Bundle {
    /// Builds the bundle of a valid contract.
    #[must_use]
    pub fn build(c: &Contract) -> Self {
        let schemas = c
            .artifacts
            .iter()
            .map(|(id, a)| {
                (
                    id.clone(),
                    json!({"kind": a.kind.as_str(), "data": a.data_value()}),
                )
            })
            .collect();
        Self {
            contract: canonical(c),
            schemas,
            extras: BTreeMap::new(),
        }
    }

    /// The fingerprint of the contract inside.
    #[must_use]
    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint::of_bytes(&jcs(&self.contract))
    }

    /// The bundle's JSON value.
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({
            "contract": self.contract,
            "schemas": Value::Object(self.schemas.clone().into_iter().collect::<Map<_, _>>()),
            "extras": Value::Object(self.extras.clone().into_iter().collect::<Map<_, _>>()),
        })
    }

    /// The bundle file's bytes: JCS of [`Self::to_value`].
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        jcs(&self.to_value())
    }

    /// Parses and verifies a bundle file. On success the bundle is intact:
    /// every listed schema is present and hashes to its id, and nothing
    /// else is carried.
    pub fn verify(bytes: &[u8]) -> Result<Self, BundleError> {
        let text = std::str::from_utf8(bytes).map_err(|e| BundleError::Shape(e.to_string()))?;
        let v = crate::strict::parse_json(text)?;
        let top = v.as_object().ok_or_else(|| shape("not an object"))?;
        for k in top.keys() {
            if !matches!(k.as_str(), "contract" | "schemas" | "extras") {
                return Err(shape(&format!("unknown member {k:?}")));
            }
        }
        let contract = top
            .get("contract")
            .cloned()
            .ok_or_else(|| shape("no `contract`"))?;
        let format = contract.get("format").and_then(Value::as_str).unwrap_or("");
        if format != FORMAT {
            return Err(BundleError::Format(format.to_owned()));
        }
        let mut report = Report::default();
        check_restrictions(&contract, &mut report);
        if report.has_errors() {
            return Err(BundleError::Restrictions(
                report.to_string().trim().to_owned(),
            ));
        }
        let obj = |k: &str| -> Result<BTreeMap<String, Value>, BundleError> {
            match top.get(k) {
                None => Ok(BTreeMap::new()),
                Some(Value::Object(m)) => Ok(m.clone().into_iter().collect()),
                Some(_) => Err(shape(&format!("`{k}` is not an object"))),
            }
        };
        let schemas = obj("schemas")?;
        let extras = obj("extras")?;

        let listed = contract
            .get("schemas")
            .and_then(Value::as_array)
            .ok_or_else(|| shape("the contract has no `schemas` list"))?;
        let mut want: BTreeMap<String, String> = BTreeMap::new();
        for s in listed {
            let id = s
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| shape("a schema without `id`"))?;
            let kind = s
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| shape("a schema without `kind`"))?;
            want.insert(id.to_owned(), kind.to_owned());
        }
        for id in schemas.keys() {
            if !want.contains_key(id) {
                return Err(BundleError::UnlistedSchema(id.clone()));
            }
        }
        for (id, kind) in &want {
            let entry = schemas
                .get(id)
                .ok_or_else(|| BundleError::MissingSchema(id.clone()))?;
            let got_kind = entry.get("kind").and_then(Value::as_str).unwrap_or("");
            if got_kind != kind {
                return Err(BundleError::SchemaKind {
                    id: id.clone(),
                    got: got_kind.to_owned(),
                    want: kind.clone(),
                });
            }
            let data = entry
                .get("data")
                .ok_or_else(|| shape(&format!("schema {id} has no `data`")))?;
            let got = match kind.as_str() {
                "protobuf" => {
                    let b64 = data
                        .as_str()
                        .ok_or_else(|| shape(&format!("schema {id}: data is not base64")))?;
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(b64)
                        .map_err(|e| shape(&format!("schema {id}: {e}")))?;
                    sha256_id(&bytes)
                }
                "jsonschema" => sha256_id(&jcs(data)),
                other => return Err(shape(&format!("schema {id}: unknown kind {other:?}"))),
            };
            if &got != id {
                return Err(BundleError::SchemaHash {
                    id: id.clone(),
                    got,
                });
            }
        }
        for (id, e) in &extras {
            let data = e
                .get("data")
                .ok_or_else(|| shape(&format!("extra {id} has no `data`")))?;
            let got = sha256_id(&jcs(data));
            if &got != id {
                return Err(BundleError::ExtraHash {
                    id: id.clone(),
                    got,
                });
            }
        }
        Ok(Self {
            contract,
            schemas,
            extras,
        })
    }

    /// [`Self::verify`], then checks the fingerprint against the one the
    /// caller expected (from a contract key or a descriptor).
    pub fn verify_expecting(bytes: &[u8], want: &Fingerprint) -> Result<Self, BundleError> {
        let b = Self::verify(bytes)?;
        let got = b.fingerprint();
        if &got != want {
            return Err(BundleError::Fingerprint {
                got,
                want: want.clone(),
            });
        }
        Ok(b)
    }
}

fn shape(s: &str) -> BundleError {
    BundleError::Shape(s.to_owned())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::contract::load_str;

    fn sample() -> Contract {
        let src = r#"
[interface]
name = "t"
major = 1
minor = 0

[resources."a"]
kind = "operation"
request = "google.protobuf.Empty"
response = "google.protobuf.Timestamp"
"#;
        let l = load_str(src, Path::new("."), None);
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    }

    #[test]
    fn round_trip_and_tamper() {
        let c = sample();
        let b = Bundle::build(&c);
        assert_eq!(b.schemas.len(), 2, "empty.proto and timestamp.proto");
        let bytes = b.to_bytes();
        let v = Bundle::verify_expecting(&bytes, &Fingerprint::of(&c)).unwrap();
        assert_eq!(v, b);

        let text = String::from_utf8(bytes).unwrap();
        let tampered = text.replace("\"interactive_high\"", "\"real_time\"");
        let err = Bundle::verify_expecting(tampered.as_bytes(), &Fingerprint::of(&c)).unwrap_err();
        assert!(matches!(err, BundleError::Fingerprint { .. }), "{err}");

        let mut v: Value = serde_json::from_str(&text).unwrap();
        let id = b.schemas.keys().next().unwrap().clone();
        v["schemas"][&id]["data"] = json!("AAAA");
        let err = Bundle::verify(&jcs(&v)).unwrap_err();
        assert!(matches!(err, BundleError::SchemaHash { .. }), "{err}");

        v["schemas"].as_object_mut().unwrap().remove(&id);
        let err = Bundle::verify(&jcs(&v)).unwrap_err();
        assert!(matches!(err, BundleError::MissingSchema(_)), "{err}");

        let dup = text.replacen("{\"contract\":", "{\"extras\":{},\"contract\":", 1);
        assert!(matches!(
            Bundle::verify(dup.as_bytes()),
            Err(BundleError::Json(_))
        ));
    }
}

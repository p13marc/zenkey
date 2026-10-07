//! Mock values: a minimal valid instance of any contract type, so a mock
//! service can serve every resource of any example contract with no
//! hand-written code.
//!
//! - JSON Schema: the smallest instance the definition admits (required
//!   properties, first enum value, minimums), following `$ref` within a
//!   file and across listed files by stem.
//! - Protobuf: the default message, encoded (usually empty bytes).
//! - Raw: a few placeholder bytes; a family `video/*` becomes `video/mock`.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use prost_reflect::{DescriptorPool, DynamicMessage};
use serde_json::{Map, Value, json};
use zenkey_model::authoring::Encoding as Enc;
use zenkey_model::contract::Contract;
use zenkey_model::schema::{ArtifactData, TypeId};
use zenoh::bytes::Encoding;

/// A payload ready to put or reply.
#[derive(Debug, Clone)]
pub struct Payload {
    pub bytes: Vec<u8>,
    pub encoding: Encoding,
}

/// Builds mock payloads for one contract.
pub struct Mocker<'a> {
    contract: &'a Contract,
    pools: BTreeMap<String, DescriptorPool>,
}

impl<'a> Mocker<'a> {
    #[must_use]
    pub fn new(contract: &'a Contract) -> Self {
        Self { contract, pools: BTreeMap::new() }
    }

    /// A payload of type `t`, encoded as `enc` when `t` is a JSON Schema type.
    pub fn payload(&mut self, t: &TypeId, enc: Option<Enc>) -> Result<Payload> {
        match t {
            TypeId::JsonSchema { name, schema } => {
                let v = self.json_instance(schema, name)?;
                Ok(match enc.unwrap_or(Enc::Json) {
                    Enc::Json => Payload { bytes: serde_json::to_vec(&v)?, encoding: Encoding::APPLICATION_JSON },
                    Enc::Cbor => {
                        let mut b = Vec::new();
                        ciborium::into_writer(&v, &mut b).map_err(|e| anyhow!("cbor: {e}"))?;
                        Payload { bytes: b, encoding: Encoding::APPLICATION_CBOR }
                    }
                })
            }
            TypeId::Protobuf { name, schema } => {
                let pool = self.pool(schema)?;
                let desc = pool
                    .get_message_by_name(name)
                    .ok_or_else(|| anyhow!("{name} not in its descriptor set"))?;
                Ok(Payload {
                    bytes: prost_reflect::prost::Message::encode_to_vec(&DynamicMessage::new(desc)),
                    encoding: Encoding::APPLICATION_PROTOBUF,
                })
            }
            TypeId::Raw { media_type, .. } => Ok(Payload {
                bytes: b"zk2-spike mock".to_vec(),
                encoding: Encoding::from(media_type.replace('*', "mock")),
            }),
        }
    }

    fn pool(&mut self, schema: &str) -> Result<&DescriptorPool> {
        if !self.pools.contains_key(schema) {
            let a = self.contract.artifacts.get(schema).ok_or_else(|| anyhow!("no artifact {schema}"))?;
            let ArtifactData::Protobuf(bytes) = &a.data else {
                return Err(anyhow!("{schema} is not a protobuf artifact"));
            };
            let pool = DescriptorPool::decode(bytes.as_slice())?;
            self.pools.insert(schema.to_owned(), pool);
        }
        Ok(&self.pools[schema])
    }

    fn doc(&self, schema: &str) -> Result<&Value> {
        match self.contract.artifacts.get(schema).map(|a| &a.data) {
            Some(ArtifactData::Json(v)) => Ok(v),
            _ => Err(anyhow!("{schema} is not a JSON Schema artifact")),
        }
    }

    /// The artifact id of the listed JSON Schema file with this stem.
    fn by_stem(&self, stem: &str) -> Option<&str> {
        self.contract
            .artifacts
            .iter()
            .find(|(_, a)| a.name == stem && matches!(a.data, ArtifactData::Json(_)))
            .map(|(id, _)| id.as_str())
    }

    fn json_instance(&self, schema: &str, name: &str) -> Result<Value> {
        let doc = self.doc(schema)?;
        let def = doc
            .get("$defs")
            .and_then(|d| d.get(name))
            .ok_or_else(|| anyhow!("$defs/{name} missing"))?;
        self.instance(schema, def, 0)
    }

    fn instance(&self, schema: &str, s: &Value, depth: u32) -> Result<Value> {
        if depth > 16 {
            return Ok(Value::Null);
        }
        let Some(o) = s.as_object() else {
            // `true` admits anything.
            return Ok(Value::Null);
        };
        if let Some(Value::String(r)) = o.get("$ref") {
            let (file, ptr) = r.split_once('#').unwrap_or((r.as_str(), ""));
            let target = if file.is_empty() {
                schema.to_owned()
            } else {
                let base = file.rsplit('/').next().unwrap_or(file);
                let stem = base.strip_suffix(".json").unwrap_or(base);
                self.by_stem(stem).ok_or_else(|| anyhow!("$ref {r}: no listed file {stem}"))?.to_owned()
            };
            let doc = self.doc(&target)?;
            let sub = if ptr.is_empty() { doc } else { doc.pointer(ptr).ok_or_else(|| anyhow!("$ref {r}"))? };
            return self.instance(&target, sub, depth + 1);
        }
        if let Some(c) = o.get("const") {
            return Ok(c.clone());
        }
        if let Some(e) = o.get("enum").and_then(Value::as_array).and_then(|a| a.first()) {
            return Ok(e.clone());
        }
        if let Some(d) = o.get("default") {
            return Ok(d.clone());
        }
        for k in ["oneOf", "anyOf"] {
            if let Some(first) = o.get(k).and_then(Value::as_array).and_then(|a| a.first()) {
                return self.instance(schema, first, depth + 1);
            }
        }
        if let Some(all) = o.get("allOf").and_then(Value::as_array) {
            let mut out = Map::new();
            for sub in all {
                if let Value::Object(m) = self.instance(schema, sub, depth + 1)? {
                    out.extend(m);
                }
            }
            return Ok(Value::Object(out));
        }
        let ty = match o.get("type") {
            Some(Value::String(t)) => t.as_str(),
            Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).find(|t| *t != "null").unwrap_or("null"),
            _ if o.contains_key("properties") => "object",
            _ => "null",
        };
        Ok(match ty {
            "object" => {
                let mut out = Map::new();
                let required: Vec<&str> = o
                    .get("required")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                if let Some(props) = o.get("properties").and_then(Value::as_object) {
                    for (k, sub) in props {
                        if required.contains(&k.as_str()) {
                            out.insert(k.clone(), self.instance(schema, sub, depth + 1)?);
                        }
                    }
                }
                Value::Object(out)
            }
            "array" => {
                let n = o.get("minItems").and_then(Value::as_u64).unwrap_or(0);
                let item = o.get("items").cloned().unwrap_or(Value::Bool(true));
                let mut v = Vec::new();
                for _ in 0..n {
                    v.push(self.instance(schema, &item, depth + 1)?);
                }
                Value::Array(v)
            }
            "string" => {
                let n = o.get("minLength").and_then(Value::as_u64).unwrap_or(0);
                json!("x".repeat(usize::try_from(n).unwrap_or(0)))
            }
            "integer" => json!(o.get("minimum").and_then(Value::as_i64).unwrap_or(0)),
            "number" => json!(o.get("minimum").and_then(Value::as_f64).unwrap_or(0.0)),
            "boolean" => json!(false),
            _ => Value::Null,
        })
    }
}

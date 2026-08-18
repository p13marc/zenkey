//! Schema-driven payload synthesis (#162) — the datagen half of `zenctl gen`.
//!
//! Synthesis produces a **JSON value** for every schema kind; the kind's own
//! encoder (`DecoderRegistry::encode`, the same seam `topic pub` writes
//! through) turns it into wire bytes. That keeps this module codec-free: it
//! never frames bytes, it only answers "what instance would this schema
//! accept?".
//!
//! Deterministic on purpose: a `(seed, tick)` pair always yields the same
//! instance (spray's seeded-sine precedent) — a generator whose runs cannot
//! be reproduced cannot be used to bisect a consumer bug. Numeric leaves
//! wander on a sine per field so plots move; everything else is stable.

use serde_json::{Map, Value, json};
use zenkey::schema::TypeSchema;

/// How deep nested objects/arrays are followed before giving up — a cyclic
/// or pathological schema degrades to a placeholder, not a stack overflow.
const DEPTH_CAP: usize = 6;

/// A deterministic instance generator.
#[derive(Debug, Clone, Copy)]
pub struct Synth {
    pub seed: u64,
}

/// A cheap deterministic hash for per-field phase offsets (FNV-1a) — not
/// cryptographic, just stable across runs and platforms.
fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

impl Synth {
    pub fn new(seed: u64) -> Synth {
        Synth { seed }
    }

    /// A wandering numeric value: a sine over `tick`, phase-offset by the
    /// field's name so sibling fields do not move in lockstep.
    fn wander(&self, field: &str, tick: u64, min: f64, max: f64) -> f64 {
        let phase = (fnv(field) ^ self.seed) % 628 /* 2π·100 */;
        let x = (tick as f64) / 10.0 + (phase as f64) / 100.0;
        let mid = f64::midpoint(min, max);
        let amp = (max - min) / 2.0;
        mid + amp * x.sin()
    }

    /// Synthesize an instance for a schema entry. `None` means this kind
    /// cannot be synthesized here (unknown kind) — the caller degrades with
    /// a stated note, never silently.
    pub fn instance(&self, schema: &TypeSchema, tick: u64) -> Option<Value> {
        match schema.kind().as_str() {
            zenkey::schema::SchemaKind::JSON_SCHEMA => schema
                .json_document()
                .map(|doc| self.json_schema_value(doc, "", tick, 0)),
            zenkey::schema::SchemaKind::CDR => {
                let fields = schema.cdr_fields()?;
                let types = schema.cdr_types();
                Some(self.cdr_fields_value(fields, types, tick, 0))
            }
            #[cfg(feature = "decode-protobuf")]
            zenkey::schema::SchemaKind::PROTOBUF => self.protobuf_value(schema, tick),
            _ => None,
        }
    }

    /// Walk a draft 2020-12 document conservatively: satisfy `type`,
    /// `required` (by emitting every declared property), `enum`/`const`,
    /// and numeric bounds. Unknown or empty schemas get a wandering number —
    /// `{}` accepts anything.
    fn json_schema_value(&self, doc: &Value, field: &str, tick: u64, depth: usize) -> Value {
        if depth > DEPTH_CAP {
            return Value::Null;
        }
        if let Some(c) = doc.get("const") {
            return c.clone();
        }
        if let Some(e) = doc.get("enum").and_then(Value::as_array)
            && let Some(first) = e.first()
        {
            return first.clone();
        }
        for branch in ["oneOf", "anyOf"] {
            if let Some(b) = doc.get(branch).and_then(Value::as_array)
                && let Some(first) = b.first()
            {
                return self.json_schema_value(first, field, tick, depth + 1);
            }
        }
        let ty = doc.get("type").and_then(Value::as_str).unwrap_or("number");
        match ty {
            "object" => {
                let mut out = Map::new();
                if let Some(props) = doc.get("properties").and_then(Value::as_object) {
                    for (name, sub) in props {
                        out.insert(
                            name.clone(),
                            self.json_schema_value(sub, name, tick, depth + 1),
                        );
                    }
                }
                Value::Object(out)
            }
            "array" => {
                let n = doc
                    .get("minItems")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .max(1);
                let item = doc.get("items").cloned().unwrap_or(json!({}));
                Value::Array(
                    (0..n)
                        .map(|i| self.json_schema_value(&item, field, tick + i, depth + 1))
                        .collect(),
                )
            }
            "string" => Value::String(format!(
                "{}-{}",
                if field.is_empty() { "s" } else { field },
                tick % 10
            )),
            "boolean" => Value::Bool(tick.is_multiple_of(2)),
            "integer" => {
                let (min, max) = bounds(doc, 0.0, 100.0);
                json!(self.wander(field, tick, min, max).round() as i64)
            }
            "null" => Value::Null,
            // "number" and anything else numeric-shaped.
            _ => {
                let (min, max) = bounds(doc, 0.0, 100.0);
                json!(self.wander(field, tick, min, max))
            }
        }
    }

    /// The `cdr` kind's compact field list (RFC 08 §7.1): positional
    /// `[{name, type}]` with a local `types` table for composites.
    fn cdr_fields_value(
        &self,
        fields: &Value,
        types: Option<&Map<String, Value>>,
        tick: u64,
        depth: usize,
    ) -> Value {
        if depth > DEPTH_CAP {
            return Value::Null;
        }
        let Some(list) = fields.as_array() else {
            return Value::Null;
        };
        let mut out = Map::new();
        for f in list {
            let Some(name) = f.get("name").and_then(Value::as_str) else {
                continue;
            };
            let ty = f.get("type").cloned().unwrap_or(Value::Null);
            out.insert(
                name.to_string(),
                self.cdr_value(&ty, types, name, tick, depth),
            );
        }
        Value::Object(out)
    }

    fn cdr_value(
        &self,
        ty: &Value,
        types: Option<&Map<String, Value>>,
        field: &str,
        tick: u64,
        depth: usize,
    ) -> Value {
        if depth > DEPTH_CAP {
            return Value::Null;
        }
        match ty {
            Value::String(name) => match name.as_str() {
                "bool" => Value::Bool(tick.is_multiple_of(2)),
                "string" => Value::String(format!("{field}-{}", tick % 10)),
                "float32" | "float" | "float64" | "double" => {
                    json!(self.wander(field, tick, 0.0, 100.0))
                }
                // The full primitive vocabulary of the cdr kind (RFC 08
                // §7.1's IDL-flavoured aliases included).
                "int8" | "char" | "int16" | "short" | "int32" | "long" | "int64" | "long long" => {
                    json!(self.wander(field, tick, 0.0, 100.0).round() as i64)
                }
                "uint8" | "byte" | "octet" | "uint16" | "unsigned short" | "uint32"
                | "unsigned long" | "uint64" | "unsigned long long" => {
                    json!(self.wander(field, tick, 0.0, 100.0).round().abs() as u64)
                }
                // A named composite from the local table.
                other => match types.and_then(|t| t.get(other)) {
                    Some(composite) => {
                        let fields = composite.get("fields").unwrap_or(composite);
                        self.cdr_fields_value(fields, types, tick, depth + 1)
                    }
                    None => Value::Null,
                },
            },
            // {"array": {"of": T, "len": n}} / {"sequence": {"of": T}}
            Value::Object(o) => {
                if let Some(arr) = o.get("array") {
                    let n = arr.get("len").and_then(Value::as_u64).unwrap_or(1).max(1);
                    let of = arr.get("of").cloned().unwrap_or(Value::Null);
                    Value::Array(
                        (0..n)
                            .map(|i| self.cdr_value(&of, types, field, tick + i, depth + 1))
                            .collect(),
                    )
                } else if let Some(seq) = o.get("sequence") {
                    let of = seq.get("of").cloned().unwrap_or(Value::Null);
                    Value::Array(vec![self.cdr_value(&of, types, field, tick, depth + 1)])
                } else {
                    Value::Null
                }
            }
            _ => Value::Null,
        }
    }

    /// Protobuf: field names and kinds off the served descriptor; the value
    /// is JSON in prost-reflect's serde dialect, which `store.encode`
    /// deserializes into a `DynamicMessage`.
    #[cfg(feature = "decode-protobuf")]
    fn protobuf_value(&self, schema: &TypeSchema, tick: u64) -> Option<Value> {
        use prost_reflect::{DescriptorPool, Kind};
        let fds = schema.protobuf_descriptor_set()?;
        let message = schema.protobuf_message()?;
        let pool = DescriptorPool::decode(fds.as_slice()).ok()?;
        let desc = pool.get_message_by_name(message)?;
        fn message_value(
            synth: &Synth,
            desc: &prost_reflect::MessageDescriptor,
            tick: u64,
            depth: usize,
        ) -> Value {
            if depth > DEPTH_CAP {
                return Value::Object(Map::new());
            }
            let mut out = Map::new();
            for field in desc.fields() {
                let name = field.json_name().to_string();
                let v = match field.kind() {
                    Kind::Double | Kind::Float => json!(synth.wander(&name, tick, 0.0, 100.0)),
                    Kind::Int32
                    | Kind::Int64
                    | Kind::Sint32
                    | Kind::Sint64
                    | Kind::Sfixed32
                    | Kind::Sfixed64 => {
                        json!(synth.wander(&name, tick, 0.0, 100.0).round() as i64)
                    }
                    Kind::Uint32 | Kind::Uint64 | Kind::Fixed32 | Kind::Fixed64 => {
                        json!(synth.wander(&name, tick, 0.0, 100.0).round().abs() as u64)
                    }
                    Kind::Bool => Value::Bool(tick.is_multiple_of(2)),
                    Kind::String => Value::String(format!("{name}-{}", tick % 10)),
                    Kind::Bytes => Value::String(String::new()),
                    Kind::Enum(e) => e
                        .values()
                        .next()
                        .map(|v| Value::String(v.name().to_string()))
                        .unwrap_or(Value::Null),
                    Kind::Message(m) => message_value(synth, &m, tick, depth + 1),
                };
                let v = if field.is_list() {
                    Value::Array(vec![v])
                } else {
                    v
                };
                out.insert(name, v);
            }
            Value::Object(out)
        }
        Some(message_value(self, &desc, tick, 0))
    }
}

fn bounds(doc: &Value, dmin: f64, dmax: f64) -> (f64, f64) {
    let min = doc.get("minimum").and_then(Value::as_f64).unwrap_or(dmin);
    let max = doc
        .get("maximum")
        .and_then(Value::as_f64)
        .unwrap_or_else(|| dmax.max(min + 1.0));
    (min, max.max(min))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey::schema::WireEncoding;
    use zenkey::schema::decode::DecoderRegistry;

    /// The whole point: a synthesized instance survives the kind's own
    /// encoder — and for json-schema (with validate-json on in tests) that
    /// encoder *validates*, so this is a real conformance round trip.
    #[test]
    fn a_synthesized_json_instance_encodes_and_validates() {
        let schema = TypeSchema::json_schema(json!({
            "type": "object",
            "required": ["status", "load", "cores"],
            "properties": {
                "status": { "type": "string", "enum": ["ok", "degraded"] },
                "load": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                "cores": { "type": "integer", "minimum": 1, "maximum": 128 },
                "tags": { "type": "array", "items": { "type": "string" } },
                "nested": {
                    "type": "object",
                    "properties": { "up": { "type": "boolean" } },
                },
            },
        }));
        let registry = DecoderRegistry::new();
        let synth = Synth::new(42);
        for tick in 0..20 {
            let v = synth
                .instance(&schema, tick)
                .expect("json-schema synthesizes");
            let bytes = registry
                .encode(&schema, &v, &WireEncoding::Json)
                .unwrap_or_else(|e| panic!("tick {tick}: {v} refused: {e}"));
            let back = registry
                .decode(&schema, &WireEncoding::Json, &bytes)
                .unwrap();
            assert_eq!(
                back.verdict,
                zenkey::schema::validate::Verdict::Valid,
                "tick {tick}"
            );
        }
    }

    /// Same (seed, tick) → same instance; different tick → the numerics move.
    #[test]
    fn synthesis_is_deterministic_and_wanders() {
        let schema = TypeSchema::json_schema(json!({
            "type": "object",
            "properties": { "v": { "type": "number" } },
        }));
        let synth = Synth::new(7);
        assert_eq!(
            synth.instance(&schema, 3),
            synth.instance(&schema, 3),
            "reproducible runs are the point"
        );
        assert_ne!(synth.instance(&schema, 3), synth.instance(&schema, 4));
    }

    /// The cdr field list synthesizes an object its encoder accepts.
    #[cfg(feature = "decode-cdr")]
    #[test]
    fn a_synthesized_cdr_instance_encodes() {
        let schema = TypeSchema::cdr(json!({
            "fields": [
                { "name": "x", "type": "float64" },
                { "name": "n", "type": "uint32" },
                { "name": "label", "type": "string" },
            ],
        }));
        let registry = DecoderRegistry::new();
        let v = Synth::new(1).instance(&schema, 0).expect("cdr synthesizes");
        let bytes = registry
            .encode(&schema, &v, &WireEncoding::Cdr)
            .expect("the instance encodes");
        assert!(!bytes.is_empty());
    }

    /// An unknown kind is `None` — the caller states the degradation.
    #[test]
    fn an_unknown_kind_declines_instead_of_guessing() {
        let set = zenkey::schema::SchemaSet::parse(
            r#"{"schema_version":1,"app":"t",
                "types":{"W":{"kind":"cddl","hash":"sha256:00","spec":"x = int"}}}"#,
        )
        .unwrap();
        assert_eq!(Synth::new(0).instance(set.get("W").unwrap(), 0), None);
    }
}

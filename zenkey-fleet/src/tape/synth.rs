//! Payload synthesis from a contract (#612, FJ8a): what a mock owner
//! publishes and answers, built from the revision's bundle alone (spec §7).
//!
//! Every type kind of §7.1 has a synthesizer:
//!
//! * **JSON Schema** through the §7.3 subset: `type` (a set of names),
//!   `properties`, `required`, `additionalProperties`, `items`,
//!   `prefixItems`, `enum`, `const`, the numeric bounds, the string and
//!   array lengths, `$ref` across the listed files, `oneOf` and `anyOf`.
//!   Annotations are ignored, as the subset ignores them. The value is then
//!   checked by [`zenkey_model::validate::validate`], and one that does not
//!   pass is refused here, never sent: a mock owner writes only what its
//!   schema declares (§9.8).
//! * **protobuf** through the bundle's descriptor set: a `DynamicMessage`
//!   per message, every field filled with a value that is not its default
//!   (a default is not on the wire, and a consumer would see nothing), the
//!   first field of each `oneof`, one element per list and one entry per
//!   map. Recursion stops at a depth, leaving the deeper message unset.
//!   `google.protobuf.Any` is left unset: a value of it names a type URL no
//!   bundle resolves.
//! * **raw** as bytes of its media type's size class ([`size_class`]): no
//!   generic tool decodes a raw type (§7.1), so its bytes carry nothing but
//!   their size, deterministic noise for a binary type and a short line for
//!   `text/*`.
//!
//! **Deterministic per `(seed, tick)`**, as v1's generator was: a run that
//! cannot be reproduced cannot bisect a consumer bug. Numeric leaves wander
//! on a sine, phase-offset by the field's name so siblings do not move in
//! lockstep; everything else is stable or cycles with the tick.

use std::collections::BTreeSet;

use prost_reflect::{DescriptorPool, DynamicMessage, Kind as ProtoKind, MapKey, MessageDescriptor};
use serde_json::{Map, Value, json};
use zenkey_model::authoring::Encoding as WireEncoding;
use zenkey_model::bundle::Bundle;
use zenkey_model::contract::{Body, Resource};
use zenkey_model::schema::TypeId;
use zenkey_model::validate::{resolve_ref, satisfies, validate};

use crate::model::catalog::Revision;
use crate::model::render::Member;

/// How deep a JSON value is built before every optional part is left out:
/// past it an object carries only its required properties, an array only
/// its `minItems`, and a nullable prefers its null. What makes a recursive
/// type finite.
const SOFT_DEPTH: usize = 6;

/// How deep a JSON value is built at all: a schema whose required parts
/// recurse forever gets `null` here, and the check refuses it.
const HARD_DEPTH: usize = 32;

/// How deep protobuf messages nest: a message field past it is left unset.
const PROTO_DEPTH: usize = 6;

/// The schema that admits anything: what an array item or an undeclared
/// property is held to when nothing else is written.
static ANY_SCHEMA: Value = Value::Bool(true);

/// A deterministic instance generator.
#[derive(Debug, Clone, Copy)]
pub struct Synth {
    pub seed: u64,
}

/// One synthesized payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Synthesized {
    /// The payload, encoded as the contract carries the type (§7.2): JSON
    /// or CBOR for a JSON Schema type, the binary protobuf encoding, a raw
    /// type's bytes.
    pub bytes: Vec<u8>,
    /// The value the bytes encode, for a JSON Schema type (checked against
    /// its schema) or a protobuf one (its JSON mapping). Absent for a raw
    /// type.
    pub value: Option<Value>,
}

/// The type and the wire encoding of a resource's `member` (§7.1): a data
/// resource's `type` or `attachment`, an operation's `request`, `response`,
/// `error` or `summary`. `None` when the resource declares no such member.
#[must_use]
pub fn member_type(r: &Resource, member: Member) -> Option<(&TypeId, Option<WireEncoding>)> {
    match (&r.body, member) {
        (Body::Data(d), Member::Type) => Some((&d.type_, d.encoding)),
        (Body::Data(d), Member::Attachment) => {
            d.attachment.as_ref().map(|t| (t, d.attachment_encoding))
        }
        (Body::Operation(o), Member::Request) => Some((&o.request, o.encoding)),
        (Body::Operation(o), Member::Response) => Some((&o.response, o.encoding)),
        (Body::Operation(o), Member::Error) => o.error.as_ref().map(|t| (t, o.encoding)),
        (Body::Operation(o), Member::Summary) => o.summary.as_ref().map(|t| (t, o.encoding)),
        _ => None,
    }
}

/// The byte size a raw type's synthesized payload has: a class per
/// top-level media type, because no generic tool can do more with a raw
/// type than say its media type and size (§7.1).
#[must_use]
pub fn size_class(media_type: &str) -> usize {
    match media_type.split('/').next().unwrap_or_default() {
        "video" => 16 * 1024,
        "image" => 4 * 1024,
        "audio" => 2 * 1024,
        "text" => 64,
        _ => 256,
    }
}

/// A cheap deterministic hash for per-field phase offsets (FNV-1a): not
/// cryptographic, just stable across runs and platforms.
fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// SplitMix64: deterministic noise for a raw type's bytes.
fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

impl Synth {
    #[must_use]
    pub fn new(seed: u64) -> Synth {
        Synth { seed }
    }

    /// A wandering value in `[min, max]`: a sine over `tick`, phase-offset
    /// by the field's name.
    fn wander(&self, field: &str, tick: u64, min: f64, max: f64) -> f64 {
        let phase = (fnv(field) ^ self.seed) % 628;
        let x = (tick as f64) / 10.0 + (phase as f64) / 100.0;
        let mid = f64::midpoint(min, max);
        let amp = (max - min) / 2.0;
        (mid + amp * x.sin()).clamp(min, max)
    }

    /// One payload of `member` of `r`, at `tick`, encoded as the contract
    /// carries it. `Err` names why none could be built: a member the
    /// resource does not declare, a schema this synthesizer could not
    /// satisfy (the value it built did not validate), a descriptor set that
    /// does not decode.
    pub fn sample(
        &self,
        revision: &Revision,
        r: &Resource,
        member: Member,
        tick: u64,
    ) -> Result<Synthesized, String> {
        let (ty, encoding) = member_type(r, member)
            .ok_or_else(|| format!("{}/{} declares no {}", r.token, r.template, member.as_str()))?;
        let bundle = revision.bundle();
        let canonical = zenkey_model::decode::type_of(
            bundle,
            r.token.as_str(),
            r.template.as_str(),
            member.as_str(),
        )
        .ok_or_else(|| {
            format!(
                "the bundle holds no {} for {}/{}",
                member.as_str(),
                r.token,
                r.template
            )
        })?;
        match ty {
            TypeId::JsonSchema { .. } => {
                use zk2::codec::Codec as _;
                let value = self.json(bundle, canonical, tick)?;
                let bytes = zk2::codec::Json::<Value>::encode(&value, encoding)?;
                Ok(Synthesized {
                    bytes,
                    value: Some(value),
                })
            }
            TypeId::Protobuf { .. } => {
                let msg = self.protobuf(bundle, canonical, tick)?;
                let value = serde_json::to_value(&msg).ok();
                Ok(Synthesized {
                    bytes: zk2::prost::Message::encode_to_vec(&msg),
                    value,
                })
            }
            TypeId::Raw { media_type, .. } => Ok(Synthesized {
                bytes: self.raw(media_type, tick),
                value: None,
            }),
        }
    }

    /// A value of the JSON Schema type `ty` (a canonical type reference of
    /// `bundle`), checked against it: `Err` lists what the schema refused,
    /// and nothing that fails is ever handed out.
    pub fn json(&self, bundle: &Bundle, ty: &Value, tick: u64) -> Result<Value, String> {
        let (Some(id), Some(name)) = (ty["schema"].as_str(), ty["name"].as_str()) else {
            return Err(format!("{ty} is not a JSON Schema type reference"));
        };
        let doc = bundle
            .schemas
            .get(id)
            .map(|s| &s["data"])
            .ok_or_else(|| format!("the bundle holds no schema {id}"))?;
        let pointer = format!("#/$defs/{}", name.replace('~', "~0").replace('/', "~1"));
        let (schema, doc) = resolve_ref(bundle, doc, &pointer)
            .ok_or_else(|| format!("{name} is not defined in its schema"))?;
        let value = JsonWalk {
            synth: self,
            bundle,
        }
        .value(schema, doc, "", tick, 0);
        validate(bundle, ty, &value).map_err(|violations| {
            format!(
                "the synthesized json:{name} does not validate: {}",
                violations
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })?;
        Ok(value)
    }

    /// A message of the protobuf type `ty` (a canonical type reference of
    /// `bundle`), built through the bundle's descriptor set.
    pub fn protobuf(
        &self,
        bundle: &Bundle,
        ty: &Value,
        tick: u64,
    ) -> Result<DynamicMessage, String> {
        use base64::Engine as _;
        let name = ty["name"].as_str().unwrap_or_default();
        let set = ty["schema"]
            .as_str()
            .and_then(|id| bundle.schemas.get(id))
            .and_then(|s| s["data"].as_str())
            .and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok())
            .ok_or_else(|| format!("the bundle carries no descriptor set for {name}"))?;
        let pool = DescriptorPool::decode(set.as_slice())
            .map_err(|e| format!("the descriptor set for {name} does not decode: {e}"))?;
        let desc = pool
            .get_message_by_name(name)
            .ok_or_else(|| format!("{name} is not in the bundle's descriptor set"))?;
        Ok(self.message(&desc, tick, 0))
    }

    /// The bytes of a raw type of `media_type`, its size class long.
    #[must_use]
    pub fn raw(&self, media_type: &str, tick: u64) -> Vec<u8> {
        let n = size_class(media_type);
        if media_type.starts_with("text/") {
            let line = format!("{media_type} sample {tick} (seed {})\n", self.seed);
            return line.bytes().cycle().take(n).collect();
        }
        let base = self.seed ^ fnv(media_type) ^ tick.wrapping_mul(0x0000_0100_0000_01b3);
        (0..n.div_ceil(8) as u64)
            .flat_map(|i| splitmix(base ^ i).to_le_bytes())
            .take(n)
            .collect()
    }

    fn message(&self, desc: &MessageDescriptor, tick: u64, depth: usize) -> DynamicMessage {
        let mut msg = DynamicMessage::new(desc.clone());
        let mut oneofs: BTreeSet<String> = BTreeSet::new();
        for field in desc.fields() {
            if let Some(o) = field.containing_oneof()
                && !o.is_synthetic()
                && !oneofs.insert(o.full_name().to_owned())
            {
                continue;
            }
            let name = field.name().to_owned();
            let value = if field.is_map() {
                let ProtoKind::Message(entry) = field.kind() else {
                    continue;
                };
                let (k, v) = (entry.map_entry_key_field(), entry.map_entry_value_field());
                let (Some(key), Some(value)) = (
                    self.map_key(&k.kind(), &name, tick),
                    self.scalar(&v.kind(), &name, tick, depth),
                ) else {
                    continue;
                };
                prost_reflect::Value::Map([(key, value)].into_iter().collect())
            } else if field.is_list() {
                let Some(v) = self.scalar(&field.kind(), &name, tick, depth) else {
                    continue;
                };
                prost_reflect::Value::List(vec![v])
            } else {
                let Some(v) = self.scalar(&field.kind(), &name, tick, depth) else {
                    continue;
                };
                v
            };
            msg.set_field(&field, value);
        }
        msg
    }

    /// One value of a field's kind, never its default; `None` leaves the
    /// field unset (a message past the depth, or `Any`).
    fn scalar(
        &self,
        kind: &ProtoKind,
        name: &str,
        tick: u64,
        depth: usize,
    ) -> Option<prost_reflect::Value> {
        use prost_reflect::Value as V;
        let n = || self.wander(name, tick, 1.0, 100.0);
        let int = || n().round().max(1.0);
        Some(match kind {
            ProtoKind::Double => V::F64(n()),
            ProtoKind::Float => V::F32(n() as f32),
            ProtoKind::Int32 | ProtoKind::Sint32 | ProtoKind::Sfixed32 => V::I32(int() as i32),
            ProtoKind::Int64 | ProtoKind::Sint64 | ProtoKind::Sfixed64 => V::I64(int() as i64),
            ProtoKind::Uint32 | ProtoKind::Fixed32 => V::U32(int() as u32),
            ProtoKind::Uint64 | ProtoKind::Fixed64 => V::U64(int() as u64),
            ProtoKind::Bool => V::Bool(true),
            ProtoKind::String => V::String(format!("{name}{}", tick % 10)),
            ProtoKind::Bytes => V::Bytes(zk2::prost::bytes::Bytes::from(
                format!("{name}{}", tick % 10).into_bytes(),
            )),
            // The second value when there is one: the first is the default,
            // which the wire never carries.
            ProtoKind::Enum(e) => V::EnumNumber(
                e.values()
                    .find(|v| v.number() != 0)
                    .or_else(|| e.values().next())
                    .map_or(0, |v| v.number()),
            ),
            ProtoKind::Message(m) => {
                if depth >= PROTO_DEPTH || m.full_name() == "google.protobuf.Any" {
                    return None;
                }
                V::Message(self.message(m, tick, depth + 1))
            }
        })
    }

    fn map_key(&self, kind: &ProtoKind, name: &str, tick: u64) -> Option<MapKey> {
        let n = self.wander(name, tick, 1.0, 100.0).round().max(1.0);
        Some(match kind {
            ProtoKind::Bool => MapKey::Bool(true),
            ProtoKind::Int32 | ProtoKind::Sint32 | ProtoKind::Sfixed32 => MapKey::I32(n as i32),
            ProtoKind::Int64 | ProtoKind::Sint64 | ProtoKind::Sfixed64 => MapKey::I64(n as i64),
            ProtoKind::Uint32 | ProtoKind::Fixed32 => MapKey::U32(n as u32),
            ProtoKind::Uint64 | ProtoKind::Fixed64 => MapKey::U64(n as u64),
            ProtoKind::String => MapKey::String(format!("{name}{}", tick % 10)),
            _ => return None,
        })
    }
}

/// One JSON Schema walk over a bundle's documents.
struct JsonWalk<'s, 'b> {
    synth: &'s Synth,
    bundle: &'b Bundle,
}

/// The type names a schema position allows, in the order written: `type`
/// as a string or a list. Empty when it says nothing.
fn type_names(schema: &Value) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

/// Whether a branch of a `oneOf` or `anyOf` admits only `null`.
fn is_null_schema(schema: &Value) -> bool {
    type_names(schema) == ["null"] || schema.get("const") == Some(&Value::Null)
}

impl<'b> JsonWalk<'_, 'b> {
    /// A value `schema` (a position inside `doc`) admits, best effort; the
    /// caller checks the whole value against its type.
    fn value(
        &self,
        schema: &'b Value,
        doc: &'b Value,
        field: &str,
        tick: u64,
        depth: usize,
    ) -> Value {
        let deep = depth >= SOFT_DEPTH;
        if depth > HARD_DEPTH {
            return Value::Null;
        }
        let obj = match schema {
            Value::Bool(true) => return json!(self.synth.wander(field, tick, 0.0, 100.0)),
            Value::Object(o) => o,
            _ => return Value::Null,
        };
        if let Some(c) = obj.get("const") {
            return c.clone();
        }
        if let Some(e) = obj.get("enum").and_then(Value::as_array)
            && !e.is_empty()
        {
            // The first the whole position admits: an enum beside a `type`
            // can list values the type refuses.
            return e
                .iter()
                .find(|v| satisfies(self.bundle, doc, schema, v))
                .unwrap_or(&e[0])
                .clone();
        }
        if let Some(r) = obj.get("$ref").and_then(Value::as_str)
            && let Some((target, tdoc)) = resolve_ref(self.bundle, doc, r)
        {
            return self.value(target, tdoc, field, tick, depth + 1);
        }
        for (k, exactly_one) in [("oneOf", true), ("anyOf", false)] {
            let Some(branches) = obj.get(k).and_then(Value::as_array) else {
                continue;
            };
            return self.branch(schema, doc, branches, exactly_one, field, tick, depth);
        }
        let names = type_names(schema);
        let ty = if deep && names.contains(&"null") {
            "null"
        } else if let Some(t) = names.iter().find(|t| **t != "null") {
            t
        } else if names.contains(&"null") {
            "null"
        } else if obj.contains_key("properties") || obj.contains_key("required") {
            "object"
        } else if obj.contains_key("items") || obj.contains_key("prefixItems") {
            "array"
        } else {
            "number"
        };
        match ty {
            "object" => self.object(obj, doc, tick, depth),
            "array" => self.array(obj, doc, field, tick, depth),
            "string" => Value::String(self.string(obj, field, tick)),
            "boolean" => Value::Bool(tick.is_multiple_of(2)),
            "integer" => self.integer(obj, field, tick),
            "null" => Value::Null,
            _ => self.number(obj, field, tick),
        }
    }

    /// A value of one branch of a `oneOf` (it must match exactly one) or an
    /// `anyOf` (at least one), and of everything written beside it. A
    /// non-null branch is preferred until the soft depth, a null one past
    /// it; the first candidate that holds wins.
    #[allow(clippy::too_many_arguments)]
    fn branch(
        &self,
        schema: &'b Value,
        doc: &'b Value,
        branches: &'b [Value],
        exactly_one: bool,
        field: &str,
        tick: u64,
        depth: usize,
    ) -> Value {
        let deep = depth >= SOFT_DEPTH;
        let mut order: Vec<&Value> = branches.iter().collect();
        order.sort_by_key(|b| is_null_schema(b) != deep);
        let mut first = None;
        for b in order {
            let candidate = self.value(b, doc, field, tick, depth + 1);
            let fits = if exactly_one {
                branches
                    .iter()
                    .filter(|x| satisfies(self.bundle, doc, x, &candidate))
                    .count()
                    == 1
            } else {
                true
            };
            if fits && satisfies(self.bundle, doc, schema, &candidate) {
                return candidate;
            }
            first.get_or_insert(candidate);
        }
        first.unwrap_or(Value::Null)
    }

    fn object(
        &self,
        obj: &'b Map<String, Value>,
        doc: &'b Value,
        tick: u64,
        depth: usize,
    ) -> Value {
        let deep = depth >= SOFT_DEPTH;
        let required: BTreeSet<&str> = obj
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let mut out = Map::new();
        if let Some(props) = obj.get("properties").and_then(Value::as_object) {
            for (name, sub) in props {
                if *sub == Value::Bool(false) || (deep && !required.contains(name.as_str())) {
                    continue;
                }
                out.insert(name.clone(), self.value(sub, doc, name, tick, depth + 1));
            }
        }
        // A required property `properties` does not declare takes what
        // `additionalProperties` admits.
        let extra = obj.get("additionalProperties").unwrap_or(&ANY_SCHEMA);
        for name in required {
            if !out.contains_key(name) && *extra != Value::Bool(false) {
                out.insert(
                    name.to_owned(),
                    self.value(extra, doc, name, tick, depth + 1),
                );
            }
        }
        Value::Object(out)
    }

    fn array(
        &self,
        obj: &'b Map<String, Value>,
        doc: &'b Value,
        field: &str,
        tick: u64,
        depth: usize,
    ) -> Value {
        let deep = depth >= SOFT_DEPTH;
        let count = |k: &str| obj.get(k).and_then(Value::as_u64).map(|n| n as usize);
        let prefix: &[Value] = obj
            .get("prefixItems")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice);
        let items = obj.get("items");
        let min = count("minItems").unwrap_or(0);
        let max = count("maxItems").unwrap_or(usize::MAX);
        let mut n = if deep {
            min
        } else {
            min.max(1).max(prefix.len())
        };
        if items == Some(&Value::Bool(false)) {
            n = n.min(prefix.len()).max(min.min(prefix.len()));
        }
        let n = n.min(max);
        Value::Array(
            (0..n)
                .map(|i| {
                    let sub = prefix.get(i).or(items).unwrap_or(&ANY_SCHEMA);
                    self.value(sub, doc, field, tick + i as u64, depth + 1)
                })
                .collect(),
        )
    }

    fn string(&self, obj: &Map<String, Value>, field: &str, tick: u64) -> String {
        let count = |k: &str| obj.get(k).and_then(Value::as_u64).map(|n| n as usize);
        let mut s = format!(
            "{}-{}",
            if field.is_empty() { "s" } else { field },
            tick % 10
        );
        let min = count("minLength").unwrap_or(0);
        while s.chars().count() < min {
            s.push('x');
        }
        if let Some(max) = count("maxLength") {
            s = s.chars().take(max).collect();
        }
        s
    }

    /// The bounds a number must lie within: `minimum`/`maximum` inclusive,
    /// `exclusiveMinimum`/`exclusiveMaximum` exclusive; a side not bound is
    /// a hundred from the other, or the default range.
    fn bounds(obj: &Map<String, Value>) -> (f64, bool, f64, bool) {
        let f = |k: &str| obj.get(k).and_then(Value::as_f64);
        let (lo, lo_open) = match (f("minimum"), f("exclusiveMinimum")) {
            (Some(a), Some(b)) if b >= a => (b, true),
            (Some(a), _) => (a, false),
            (None, Some(b)) => (b, true),
            (None, None) => (f64::NAN, false),
        };
        let (hi, hi_open) = match (f("maximum"), f("exclusiveMaximum")) {
            (Some(a), Some(b)) if b <= a => (b, true),
            (Some(a), _) => (a, false),
            (None, Some(b)) => (b, true),
            (None, None) => (f64::NAN, false),
        };
        match (lo.is_nan(), hi.is_nan()) {
            (true, true) => (0.0, false, 100.0, false),
            (false, true) => (lo, lo_open, lo + 100.0, false),
            (true, false) => (hi - 100.0, false, hi, hi_open),
            (false, false) => (lo, lo_open, hi, hi_open),
        }
    }

    fn integer(&self, obj: &Map<String, Value>, field: &str, tick: u64) -> Value {
        let (lo, lo_open, hi, hi_open) = Self::bounds(obj);
        let mut lo_i = lo.ceil();
        if lo_open && lo_i <= lo {
            lo_i += 1.0;
        }
        let mut hi_i = hi.floor();
        if hi_open && hi_i >= hi {
            hi_i -= 1.0;
        }
        if lo_i > hi_i {
            return json!(lo_i as i64);
        }
        let v = self
            .synth
            .wander(field, tick, lo_i, hi_i)
            .round()
            .clamp(lo_i, hi_i);
        if v >= 0.0 {
            json!(v as u64)
        } else {
            json!(v as i64)
        }
    }

    fn number(&self, obj: &Map<String, Value>, field: &str, tick: u64) -> Value {
        let (lo, lo_open, hi, hi_open) = Self::bounds(obj);
        let eps = ((hi - lo).abs() * 1e-6).max(1e-9);
        let lo = if lo_open { lo + eps } else { lo };
        let hi = if hi_open { hi - eps } else { hi };
        let v = if lo <= hi {
            self.synth.wander(field, tick, lo, hi)
        } else {
            lo
        };
        serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::ContractSource;
    use zenkey_model::decode::{Rendered, decode};

    /// A contract from `toml` and its schema files, in a directory of its
    /// own.
    fn revision(tag: &str, toml: &str, files: &[(&str, &str)]) -> Revision {
        let dir =
            std::env::temp_dir().join(format!("zenkey-fleet-synth-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        for (name, text) in files {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("dirs");
            std::fs::write(path, text).expect("write");
        }
        let l = zenkey_model::contract::load_str(toml, &dir, None);
        let c = l.contract.unwrap_or_else(|| panic!("{}", l.report));
        Revision::from_contract(c, ContractSource::File)
    }

    const SUBSET: &str = r##"{"$defs": {
        "Status": {
            "type": "object",
            "properties": {
                "state": {"type": "string", "enum": ["up", "down"]},
                "kind": {"const": "status"},
                "since_ms": {"type": "integer", "minimum": 0},
                "load": {"type": "number", "exclusiveMinimum": 0, "exclusiveMaximum": 1},
                "cores": {"type": "integer", "minimum": 1, "maximum": 4},
                "label": {"type": "string", "minLength": 2, "maxLength": 3},
                "link": {"$ref": "common.json#/$defs/Link"},
                "hops": {"type": "array", "prefixItems": [{"type": "string"}],
                         "items": {"type": "integer"}, "minItems": 2, "maxItems": 3},
                "pair": {"type": "array", "prefixItems": [{"type": "boolean"}, {"type": "null"}],
                         "items": false},
                "retries": {"anyOf": [{"type": "integer"}, {"type": "null"}]},
                "nullable": {"type": ["null", "string"]},
                "event": {"oneOf": [
                    {"type": "object", "properties": {"up": {"const": true}}, "required": ["up"]},
                    {"type": "object", "properties": {"down": {"const": true}}, "required": ["down"]}
                ]},
                "loose": {"oneOf": [{"type": "string"}, {"type": "string", "maxLength": 1}]},
                "tree": {"$ref": "#/$defs/Tree"},
                "any": true,
                "never": false
            },
            "required": ["state", "kind", "tags"],
            "additionalProperties": {"type": "array", "items": {"type": "string"}}
        },
        "Tree": {
            "type": "object",
            "properties": {"label": {"type": "string"},
                           "children": {"type": "array", "items": {"$ref": "#/$defs/Tree"}}},
            "required": ["label"]
        }
    }}"##;

    const COMMON: &str = r#"{"$defs": {"Link": {"type": "object",
        "properties": {"mbps": {"type": "number", "minimum": 10, "maximum": 20}},
        "required": ["mbps"], "additionalProperties": false}}}"#;

    const PROTO: &str = r#"syntax = "proto3";
package m.v1;
import "google/protobuf/timestamp.proto";
import "google/protobuf/any.proto";
enum Mode { MODE_UNSPECIFIED = 0; MODE_FAST = 1; }
message Inner { int32 n = 1; repeated Inner more = 2; }
message Pose {
  double x = 1; float y = 2; int64 seq = 3; uint32 count = 4; sint32 delta = 5;
  fixed64 big = 6; bool ok = 7; string frame = 8; bytes blob = 9; Mode mode = 10;
  Inner inner = 11; repeated string tags = 12; map<string, int32> counts = 13;
  oneof choice { string name = 14; int32 id = 15; }
  optional double maybe = 16;
  google.protobuf.Timestamp at = 17;
  google.protobuf.Any extra = 18;
}
"#;

    fn subset() -> Revision {
        revision(
            "subset",
            "[interface]\nname = \"m\"\nmajor = 1\nminor = 0\n\
             [schemas]\njsonschema = [\"s.json\", \"common.json\"]\nprotobuf = [\"m.proto\"]\n\
             [resources.status]\nkind = \"state\"\ntype = \"json:Status\"\n\
             [resources.cbor]\nkind = \"stream\"\ntype = \"json:Status\"\nencoding = \"cbor\"\n\
             [resources.pose]\nkind = \"stream\"\ntype = \"m.v1.Pose\"\n\
             [resources.frame]\nkind = \"stream\"\ntype = { raw = \"image/jpeg\" }\n\
             attachment = \"m.v1.Inner\"\n\
             [resources.note]\nkind = \"event\"\ntype = { raw = \"text/plain\" }\n\
             rate = \"low\"\nretention = \"1h\"\n\
             [resources.set]\nkind = \"operation\"\nrequest = \"json:Tree\"\n\
             response = \"m.v1.Inner\"\nerror = \"json:Status\"\n",
            &[
                ("s.json", SUBSET),
                ("common.json", COMMON),
                ("m.proto", PROTO),
            ],
        )
    }

    fn resource<'r>(rev: &'r Revision, template: &str) -> &'r Resource {
        rev.contract()
            .resources
            .iter()
            .find(|r| r.template.as_str() == template)
            .expect("a resource")
    }

    /// Every keyword of the §7.3 subset, satisfied: each value passes
    /// `validate`, for every tick, and the CBOR encoding carries the same
    /// value.
    #[test]
    fn a_json_schema_value_passes_validate_for_every_tick() {
        let rev = subset();
        let synth = Synth::new(42);
        for tick in 0..40 {
            let s = synth
                .sample(&rev, resource(&rev, "status"), Member::Type, tick)
                .unwrap_or_else(|e| panic!("tick {tick}: {e}"));
            let v = s.value.clone().expect("a JSON value");
            assert_eq!(v["kind"], "status");
            assert!(v.get("never").is_none(), "a `false` property is left out");
            assert!(
                v["tags"].is_array(),
                "a required property `properties` lacks"
            );
            let load = v["load"].as_f64().unwrap();
            assert!(load > 0.0 && load < 1.0, "exclusive bounds: {load}");
            assert_eq!(serde_json::from_slice::<Value>(&s.bytes).unwrap(), v);
            let cbor = synth
                .sample(&rev, resource(&rev, "cbor"), Member::Type, tick)
                .unwrap();
            let back: Value = ciborium::from_reader(cbor.bytes.as_slice()).unwrap();
            assert_eq!(back, cbor.value.unwrap());
        }
        // The recursive type ends: past the soft depth only what is
        // required, and `children` is not.
        let set = resource(&rev, "set");
        let tree = synth.sample(&rev, set, Member::Request, 3).unwrap();
        assert!(tree.value.unwrap()["label"].is_string());
    }

    /// Same `(seed, tick)`, same bytes; another tick moves the numbers;
    /// another seed moves them too.
    #[test]
    fn synthesis_is_deterministic_per_seed_and_tick() {
        let rev = subset();
        for template in ["status", "pose", "frame", "note"] {
            let r = resource(&rev, template);
            let a = Synth::new(7).sample(&rev, r, Member::Type, 3).unwrap();
            let b = Synth::new(7).sample(&rev, r, Member::Type, 3).unwrap();
            assert_eq!(a, b, "{template}: reproducible runs are the point");
        }
        let r = resource(&rev, "status");
        let at = |seed, tick| {
            Synth::new(seed)
                .sample(&rev, r, Member::Type, tick)
                .unwrap()
        };
        assert_ne!(at(7, 3).value, at(7, 4).value, "the tick moves the numbers");
        assert_ne!(at(7, 3).value, at(8, 3).value, "and so does the seed");
    }

    /// Every field filled, none at its default, and the bytes decode
    /// through the bundle as `zenkey_model::decode` decodes them.
    #[test]
    fn a_protobuf_message_fills_every_field_and_decodes() {
        let rev = subset();
        let r = resource(&rev, "pose");
        let s = Synth::new(1).sample(&rev, r, Member::Type, 5).unwrap();
        let ty = zenkey_model::decode::type_of(rev.bundle(), "stream", "pose", "type").unwrap();
        let Rendered::Value(v) = decode(rev.bundle(), ty, Some("application/protobuf"), &s.bytes)
        else {
            panic!("the message decodes")
        };
        for field in [
            "x", "y", "seq", "count", "delta", "big", "ok", "frame", "blob", "mode", "inner",
            "tags", "counts", "name", "maybe", "at",
        ] {
            assert!(v.get(field).is_some(), "{field} is filled: {v}");
        }
        assert!(v.get("id").is_none(), "one field of a oneof");
        assert!(v.get("extra").is_none(), "Any is left unset");
        assert_eq!(v["mode"], "MODE_FAST", "not the default");
        assert!(
            v["inner"]["more"][0]["more"].is_array(),
            "nesting is followed"
        );

        // An attachment and an operation's members, by kind.
        let frame = resource(&rev, "frame");
        let a = Synth::new(1)
            .sample(&rev, frame, Member::Attachment, 0)
            .unwrap();
        let ty =
            zenkey_model::decode::type_of(rev.bundle(), "stream", "frame", "attachment").unwrap();
        assert!(matches!(
            decode(rev.bundle(), ty, None, &a.bytes),
            Rendered::Value(_)
        ));
        let set = resource(&rev, "set");
        for member in [Member::Request, Member::Response, Member::Error] {
            let s = Synth::new(1).sample(&rev, set, member, 0).unwrap();
            let ty =
                zenkey_model::decode::type_of(rev.bundle(), "@op", "set", member.as_str()).unwrap();
            assert!(
                matches!(decode(rev.bundle(), ty, None, &s.bytes), Rendered::Value(_)),
                "{member:?}"
            );
        }
        let e = Synth::new(1)
            .sample(&rev, set, Member::Summary, 0)
            .unwrap_err();
        assert!(e.contains("declares no summary"), "{e}");
    }

    /// A raw type is bytes of its size class: noise for a binary type, a
    /// line for text, and the model renders it as its media type and size.
    #[test]
    fn a_raw_type_is_bytes_of_its_size_class() {
        let rev = subset();
        let frame = Synth::new(9)
            .sample(&rev, resource(&rev, "frame"), Member::Type, 2)
            .unwrap();
        assert_eq!(frame.bytes.len(), size_class("image/jpeg"));
        assert_eq!(frame.value, None);
        assert!(frame.bytes.iter().any(|b| *b != frame.bytes[0]), "noise");
        let note = Synth::new(9)
            .sample(&rev, resource(&rev, "note"), Member::Type, 2)
            .unwrap();
        assert_eq!(note.bytes.len(), size_class("text/plain"));
        assert!(std::str::from_utf8(&note.bytes).is_ok());
        let ty = zenkey_model::decode::type_of(rev.bundle(), "stream", "frame", "type").unwrap();
        assert_eq!(
            decode(rev.bundle(), ty, None, &frame.bytes),
            Rendered::Opaque {
                media_type: "image/jpeg".into(),
                size: size_class("image/jpeg")
            }
        );
        assert_eq!(size_class("video/h264"), 16 * 1024);
        assert_eq!(size_class("application/octet-stream"), 256);
    }

    /// Every member of every resource of every example contract
    /// (`examples/zk2`): a JSON value that validates, a protobuf message
    /// that decodes, a raw type of its size.
    #[test]
    fn every_example_contract_synthesizes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2");
        let mut checked = 0;
        for dir in ["tcgui", "walkthrough", "zensight", "zenoh-modem"] {
            // A bindings file beside the contracts is a problem to skip, not
            // a contract.
            let (set, _) = crate::model::catalog::ContractSet::load_path(&root.join(dir));
            assert!(!set.is_empty(), "{dir} holds contracts");
            for rev in set.iter() {
                for r in &rev.contract().resources {
                    for member in [
                        Member::Type,
                        Member::Attachment,
                        Member::Request,
                        Member::Response,
                        Member::Error,
                        Member::Summary,
                    ] {
                        let Some(_) = member_type(r, member) else {
                            continue;
                        };
                        for tick in [0, 7] {
                            let s =
                                Synth::new(3)
                                    .sample(rev, r, member, tick)
                                    .unwrap_or_else(|e| {
                                        panic!("{} {}/{}: {e}", rev.iface(), r.token, r.template)
                                    });
                            let ty = zenkey_model::decode::type_of(
                                rev.bundle(),
                                r.token.as_str(),
                                r.template.as_str(),
                                member.as_str(),
                            )
                            .unwrap();
                            let rendered = decode(rev.bundle(), ty, None, &s.bytes);
                            assert!(
                                matches!(rendered, Rendered::Value(_) | Rendered::Opaque { .. }),
                                "{} {}/{} {member:?}: {rendered:?}",
                                rev.iface(),
                                r.token,
                                r.template
                            );
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(checked > 50, "the examples were walked: {checked}");
    }
}

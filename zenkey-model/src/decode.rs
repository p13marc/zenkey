//! Decoding a payload from a bundle alone (spec `core.md` §7.1–§7.2), for
//! tools that were never compiled against the contract.
//!
//! - **Decode order:** the sample's `Encoding`, then the contract's type,
//!   then sniffing.
//! - **protobuf** decodes through the bundle's `FileDescriptorSet`
//!   (prost-reflect), without codegen; **jsonschema** types are JSON or CBOR.
//!   Both render field by field, as JSON.
//! - **Honest rendering:** a `raw` type renders as its declared media type
//!   and the sample's size, and bytes that do not decode as their declared
//!   type render as that type, the size and the reason. Neither garbage nor
//!   nothing.

use base64::Engine as _;
use prost_reflect::{DescriptorPool, DynamicMessage};
use serde_json::Value;

use crate::bundle::Bundle;

/// What a payload renders as.
#[derive(Debug, Clone, PartialEq)]
pub enum Rendered {
    /// Decoded, field by field, as JSON.
    Value(Value),
    /// A type this decoder does not decode: its declared media type and the
    /// payload's size.
    Opaque { media_type: String, size: usize },
    /// Bytes that do not decode as their declared type.
    Undecodable {
        declared: String,
        size: usize,
        reason: String,
    },
}

/// The canonical type reference of a resource's member (`type`,
/// `attachment`, `request`, `response`, `error` or `summary`), found by the
/// resource's kind token and template.
#[must_use]
pub fn type_of<'b>(
    bundle: &'b Bundle,
    token: &str,
    template: &str,
    member: &str,
) -> Option<&'b Value> {
    bundle.contract["resources"]
        .as_array()?
        .iter()
        .find(|r| r["token"] == token && r["template"] == template)
        .map(|r| &r[member])
        .filter(|t| t.is_object())
}

/// The JSON-Schema-type wire encoding of a resource member: the contract's
/// `encoding` (or `attachment_encoding` for an attachment), JSON by default.
fn contract_encoding(bundle: &Bundle, ty: &Value) -> Option<String> {
    // The type reference itself does not carry the encoding; find the
    // resource that holds this exact reference.
    bundle.contract["resources"]
        .as_array()?
        .iter()
        .find_map(|r| {
            if &r["attachment"] == ty {
                r["attachment_encoding"].as_str().map(str::to_owned)
            } else if ["type", "request", "response", "error", "summary"]
                .iter()
                .any(|m| &r[*m] == ty)
            {
                r["encoding"].as_str().map(str::to_owned)
            } else {
                None
            }
        })
}

/// Decodes `bytes` of the type `ty` (a canonical type reference of
/// `bundle`). `encoding` is the sample's `Encoding` as zenoh spells it
/// (`application/json`, `application/cbor`, `application/protobuf`, …), if
/// any.
#[must_use]
pub fn decode(bundle: &Bundle, ty: &Value, encoding: Option<&str>, bytes: &[u8]) -> Rendered {
    let size = bytes.len();
    let name = ty["name"].as_str().unwrap_or_default();
    match ty["kind"].as_str() {
        Some("raw") => {
            // The declared media type; for a raw family (`video/*`), the
            // concrete subtype the sample's `Encoding` carries.
            let declared = ty["media_type"].as_str().unwrap_or_default();
            let media_type = match encoding {
                Some(e) if declared.ends_with("/*") => e.to_owned(),
                _ => declared.to_owned(),
            };
            Rendered::Opaque { media_type, size }
        }
        Some("protobuf") => {
            let declared = name.to_owned();
            let fail = |reason: String| Rendered::Undecodable {
                declared: declared.clone(),
                size,
                reason,
            };
            let Some(pool) = artifact(bundle, ty).and_then(|data| {
                let b = base64::engine::general_purpose::STANDARD
                    .decode(data.as_str()?)
                    .ok()?;
                DescriptorPool::decode(b.as_slice()).ok()
            }) else {
                return fail("the bundle's descriptor set does not decode".to_owned());
            };
            let Some(desc) = pool.get_message_by_name(name) else {
                return fail(format!("{name} is not in the bundle's descriptor set"));
            };
            match DynamicMessage::decode(desc, bytes) {
                Ok(m) => match serde_json::to_value(&m) {
                    Ok(v) => Rendered::Value(v),
                    Err(e) => fail(e.to_string()),
                },
                Err(e) => fail(e.to_string()),
            }
        }
        Some("jsonschema") => {
            let wire = match encoding {
                Some(e) if e.starts_with("application/cbor") => "cbor".to_owned(),
                Some(e) if e.starts_with("application/json") => "json".to_owned(),
                _ => contract_encoding(bundle, ty).unwrap_or_else(|| "json".to_owned()),
            };
            let decoded = if wire == "cbor" {
                ciborium::from_reader::<ciborium::Value, _>(bytes)
                    .map_err(|e| e.to_string())
                    .and_then(|v| cbor_to_json(&v))
            } else {
                serde_json::from_slice::<Value>(bytes).map_err(|e| e.to_string())
            };
            match decoded {
                Ok(v) => Rendered::Value(v),
                Err(reason) => Rendered::Undecodable {
                    declared: format!("json:{name} ({wire})"),
                    size,
                    reason,
                },
            }
        }
        _ => Rendered::Undecodable {
            declared: ty.to_string(),
            size,
            reason: "not a type reference".to_owned(),
        },
    }
}

fn artifact<'b>(bundle: &'b Bundle, ty: &Value) -> Option<&'b Value> {
    bundle
        .schemas
        .get(ty["schema"].as_str()?)
        .map(|s| &s["data"])
}

/// CBOR as JSON, with byte strings as base64 (§7.2: bytes are CBOR byte
/// strings, or base64 in JSON).
fn cbor_to_json(v: &ciborium::Value) -> Result<Value, String> {
    use ciborium::Value as C;
    Ok(match v {
        C::Null => Value::Null,
        C::Bool(b) => Value::Bool(*b),
        C::Integer(i) => {
            let n: i128 = (*i).into();
            i64::try_from(n)
                .map(Value::from)
                .or_else(|_| u64::try_from(n).map(Value::from))
                .map_err(|_| format!("integer {n} out of range"))?
        }
        C::Float(f) => serde_json::Number::from_f64(*f)
            .map(Value::Number)
            .ok_or_else(|| format!("{f} has no JSON form"))?,
        C::Text(s) => Value::String(s.clone()),
        C::Bytes(b) => Value::String(base64::engine::general_purpose::STANDARD.encode(b)),
        C::Array(a) => Value::Array(a.iter().map(cbor_to_json).collect::<Result<_, _>>()?),
        C::Map(m) => {
            let mut out = serde_json::Map::new();
            for (k, v) in m {
                let C::Text(k) = k else {
                    return Err("a map key that is not text".to_owned());
                };
                out.insert(k.clone(), cbor_to_json(v)?);
            }
            Value::Object(out)
        }
        C::Tag(_, inner) => cbor_to_json(inner)?,
        _ => return Err("an unsupported CBOR value".to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::{Rendered, decode};
    use crate::bundle::Bundle;
    use serde_json::json;

    fn bundle(toml: &str, files: &[(&str, &str)]) -> Bundle {
        let dir =
            std::env::temp_dir().join(format!("zk2-decode-{}-{}", std::process::id(), toml.len()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in files {
            std::fs::write(dir.join(name), text).unwrap();
        }
        let l = crate::contract::load_str(toml, &dir, None);
        let c = l.contract.unwrap_or_else(|| panic!("{}", l.report));
        Bundle::build(&c)
    }

    #[test]
    fn protobuf_json_cbor_and_raw_render_honestly() {
        let b = bundle(
            "[interface]\nname = \"m\"\nmajor = 1\nminor = 0\n\
             [schemas]\nprotobuf = [\"m.proto\"]\njsonschema = [\"s.json\"]\n\
             [resources.pose]\nkind = \"stream\"\ntype = \"m.v1.Pose\"\n\
             [resources.status]\nkind = \"state\"\ntype = \"json:Status\"\nencoding = \"cbor\"\n\
             [resources.frame]\nkind = \"stream\"\ntype = { raw = \"application/x-flatbuffers\" }\n",
            &[
                (
                    "m.proto",
                    "syntax = \"proto3\";\npackage m.v1;\nmessage Pose { double x = 1; string frame = 2; }\n",
                ),
                (
                    "s.json",
                    r#"{"$defs": {"Status": {"type": "object", "properties": {"up": {"type": "boolean"}}}}}"#,
                ),
            ],
        );
        // protobuf: x = 1.5 (field 1, fixed64), frame = "map" (field 2).
        let mut pose = vec![0x09];
        pose.extend_from_slice(&1.5f64.to_le_bytes());
        pose.extend_from_slice(&[0x12, 3, b'm', b'a', b'p']);
        let ty = super::type_of(&b, "stream", "pose", "type").unwrap();
        assert_eq!(
            decode(&b, ty, Some("application/protobuf"), &pose),
            Rendered::Value(json!({"x": 1.5, "frame": "map"}))
        );
        assert!(matches!(
            decode(&b, ty, None, &[0xff, 0xff]),
            Rendered::Undecodable { size: 2, .. }
        ));

        // jsonschema, CBOR on the wire (the contract's encoding when the
        // sample says nothing).
        let mut cbor = Vec::new();
        ciborium::into_writer(&json!({"up": true}), &mut cbor).unwrap();
        let ty = super::type_of(&b, "state", "status", "type").unwrap();
        assert_eq!(
            decode(&b, ty, None, &cbor),
            Rendered::Value(json!({"up": true}))
        );
        assert_eq!(
            decode(&b, ty, Some("application/json"), br#"{"up":false}"#),
            Rendered::Value(json!({"up": false})),
            "the sample's Encoding comes first"
        );

        // raw: the declared type and the size.
        let ty = super::type_of(&b, "stream", "frame", "type").unwrap();
        assert_eq!(
            decode(&b, ty, None, &[0; 42]),
            Rendered::Opaque {
                media_type: "application/x-flatbuffers".to_owned(),
                size: 42
            }
        );
    }
}

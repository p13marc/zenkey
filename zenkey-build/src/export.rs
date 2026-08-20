//! Registry slices as documents (issue #208).
//!
//! Registry in, document out — the same shape as codegen and the RFC 08 §5
//! lints, which is why this lives beside them rather than in a CLI or in the
//! bus engine. `zenkey::slice::to_toml` made this migration first; AsyncAPI
//! follows it.
//!
//! Behind the `export` feature, because it needs `serde_json` and this crate
//! runs in consumers' build scripts, where the manifest promises the
//! dependency graph stays light.

use zenkey::RegistrySlice;

/// An AsyncAPI 3.0 document from the slice set — the prior-art direction
/// RFC 10 records, made concrete.
///
/// The mapping is deliberately literal: a subject is a channel whose address
/// is its **key pattern**, and a procedure is an operation. Nothing is
/// invented — an undeclared field is absent, not defaulted, and the `x-zenkey`
/// extensions carry the convention's own vocabulary rather than bending it
/// into AsyncAPI words that mean something else.
pub fn asyncapi(slices: &[&RegistrySlice]) -> serde_json::Value {
    let mut channels = serde_json::Map::new();
    let mut operations = serde_json::Map::new();

    for slice in slices {
        let origin = slice.service_origin.as_deref().unwrap_or("{origin}");
        for d in &slice.subjects {
            let address = match &slice.service_origin {
                // A service origin has no producer chunk (RFC 06 §5).
                Some(_) => format!("v1/{origin}/{}/{}", d.class, d.path),
                None => format!("v1/{origin}/{}/{}/{}", d.class, slice.name, d.path),
            };
            let id = format!("{}.{}", slice.name, d.path.replace('/', "."));
            let mut channel = serde_json::Map::new();
            channel.insert("address".into(), serde_json::Value::String(address));
            if let Some(desc) = &d.description {
                channel.insert(
                    "description".into(),
                    serde_json::Value::String(desc.clone()),
                );
            }
            if !d.type_name.is_empty() {
                channel.insert(
                    "messages".into(),
                    serde_json::json!({
                        d.type_name.clone(): { "name": d.type_name.clone() }
                    }),
                );
            }
            let mut ext = serde_json::Map::new();
            ext.insert("class".into(), serde_json::Value::String(d.class.clone()));
            for (k, v) in [
                ("qos", d.qos.clone()),
                ("unit", d.unit.clone()),
                ("rate", d.rate.clone()),
                ("encoding", d.encoding.clone()),
                ("since", d.since.clone()),
            ] {
                if let Some(v) = v {
                    ext.insert(k.into(), serde_json::Value::String(v));
                }
            }
            if let Some(t) = d.ttl_s {
                ext.insert("ttl_s".into(), serde_json::Value::from(t));
            }
            if let Some(c) = d.cardinality {
                ext.insert("cardinality".into(), serde_json::Value::from(c));
            }
            channel.insert("x-zenkey".into(), serde_json::Value::Object(ext));
            channels.insert(id, serde_json::Value::Object(channel));
        }

        for p in &slice.procedures {
            let address = match &slice.service_origin {
                Some(_) => format!("v1/{origin}/@rpc/{}", p.path),
                None => format!("v1/{origin}/@rpc/{}/{}", slice.name, p.path),
            };
            let id = format!("{}.@rpc.{}", slice.name, p.path.replace('/', "."));
            channels.insert(
                id.clone(),
                serde_json::json!({
                    "address": address,
                    "x-zenkey": { "plane": "@rpc", "kind": p.kind },
                }),
            );
            let mut op = serde_json::Map::new();
            // RFC 05: a `read` procedure is a GET the caller sends, a `write`
            // likewise — both are `send` from the explorer's side.
            op.insert("action".into(), serde_json::Value::String("send".into()));
            op.insert(
                "channel".into(),
                serde_json::json!({ "$ref": format!("#/channels/{id}") }),
            );
            if let Some(desc) = &p.description {
                op.insert(
                    "description".into(),
                    serde_json::Value::String(desc.clone()),
                );
            }
            let mut ext = serde_json::Map::new();
            ext.insert("kind".into(), serde_json::Value::String(p.kind.clone()));
            for (k, v) in [
                ("request", p.request.clone()),
                ("reply", p.reply.clone()),
                ("fanout", p.fanout.clone()),
                ("encoding", p.encoding.clone()),
                ("since", p.since.clone()),
            ] {
                if let Some(v) = v {
                    ext.insert(k.into(), serde_json::Value::String(v));
                }
            }
            if let Some(i) = p.idempotent {
                ext.insert("idempotent".into(), serde_json::Value::Bool(i));
            }
            op.insert("x-zenkey".into(), serde_json::Value::Object(ext));
            operations.insert(
                format!("{}.{}", slice.name, p.path.replace('/', ".")),
                serde_json::Value::Object(op),
            );
        }
    }

    let app = slices.first().map(|s| s.app.as_str()).unwrap_or("zenkey");
    serde_json::json!({
        "asyncapi": "3.0.0",
        "info": {
            "title": format!("{app} — keyspace-v2 registry"),
            "version": slices.first().map(|s| s.version.clone()).unwrap_or_default(),
            "description":
                "Generated from RFC 08 registry slices by zenkey-build. Channel addresses \
                 are base-relative key patterns; `{origin}` is the publishing identity \
                 (RFC 06 §1), not an AsyncAPI parameter.",
        },
        "channels": channels,
        "operations": operations,
    })
}

/// One type's contribution to a JSON Schema bundle: its name, and its document
/// when the producer served one this bundle can carry.
pub struct BundledType<'a> {
    pub name: &'a str,
    /// The served JSON Schema, or `None` for a schema of some other kind.
    pub document: Option<&'a serde_json::Value>,
    /// How the producer described it — `protobuf`, `cdr` — used to say what
    /// the bundle could *not* translate, rather than translating it anyway.
    pub kind: &'a str,
    pub hash: &'a str,
}

/// Assemble a JSON Schema bundle from what a fleet served.
///
/// The document-shaping half of `registry export --as jsonschema`. The fetch
/// stays where the session is (`zenkey-fleet`); this is what turns the answers
/// into a document, and it is the same registry-in-document-out shape as
/// [`asyncapi`] (issue #208).
///
/// A schema of a kind this bundle cannot express is recorded as *what it is*
/// rather than translated into a JSON Schema it is not — RFC 09 §5.1 **O4**
/// applied to a document. `undescribed` names the producers that served no
/// `describe` at all, which is not the same as having no types.
pub fn json_schema_bundle(types: &[BundledType<'_>], undescribed: &[String]) -> serde_json::Value {
    let mut defs = serde_json::Map::new();
    for t in types {
        match t.document {
            Some(doc) => {
                defs.insert(t.name.to_string(), doc.clone());
            }
            None => {
                defs.insert(
                    t.name.to_string(),
                    serde_json::json!({
                        "$comment": format!(
                            "served as kind {:?} ({}), which this bundle does not translate",
                            t.kind, t.hash
                        ),
                    }),
                );
            }
        }
    }
    let mut doc = serde_json::Map::new();
    doc.insert(
        "$schema".into(),
        serde_json::Value::String("https://json-schema.org/draft/2020-12/schema".into()),
    );
    doc.insert("$defs".into(), serde_json::Value::Object(defs));
    if !undescribed.is_empty() {
        doc.insert(
            "$comment".into(),
            serde_json::Value::String(format!(
                "producers serving no `describe`, therefore absent here: {}",
                undescribed.join(", ")
            )),
        );
    }
    serde_json::Value::Object(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVED: &str = r#"
[registry]
version = "2.0"
app = "t"
convention = 1
[producer]
name = "netring"
[[subject]]
path = "flows"
class = "telemetry"
type = "TelemetryPoint"
[[subject]]
path = "brand/new"
class = "telemetry"
type = "TelemetryPoint"
"#;

    /// The AsyncAPI mapping places a producer chunk for a host producer and
    /// omits it for a service origin (RFC 06 §5) — the one place the mapping
    /// could quietly emit an unreachable address.
    #[test]
    fn asyncapi_addresses_follow_the_grammar() {
        let host = zenkey::parse_slice(SERVED).unwrap();
        let service = zenkey::parse_slice(
            r#"
[registry]
version = "1.0"
app = "t"
convention = 1
[service]
name = "catalog"
origin = "@catalog"
[[subject]]
path = "entity/{id}"
class = "state"
type = "Entity"
"#,
        )
        .unwrap();
        let doc = asyncapi(&[&host, &service]);
        let channels = doc["channels"].as_object().unwrap();
        assert_eq!(
            channels["netring.flows"]["address"],
            "v1/{origin}/telemetry/netring/flows"
        );
        assert_eq!(
            channels["catalog.entity.{id}"]["address"],
            "v1/@catalog/state/entity/{id}"
        );
    }
}

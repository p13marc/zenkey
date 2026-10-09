//! The contract model of a bundle (#611): what generated code and tools
//! (#612) build the runtime's `Implementation` or a client from, with the
//! bundle's bytes alone.
//!
//! The canonical contract inside a bundle is fully explicit (spec §9.5):
//! every default is expanded and every type resolved, so the model is read
//! back field for field, without the authoring file or its schema files.
//! What the canonical form excludes is absent from the model it gives:
//! `minor`, `summary` and every `doc` are `None`. The round trip is exact:
//! [`Bundle::build`] of the model is the bundle it came from, byte for byte.

use std::collections::BTreeMap;
use std::str::FromStr;

use base64::Engine as _;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::authoring::{Kind, ParamType};
use crate::bundle::{Bundle, BundleError};
use crate::canonical::FORMAT;
use crate::contract::{
    Body, Contract, Data, HistoryParams, Operation, Rate, Requirement, Resource, token_of,
};
use crate::grammar::{IfaceId, KindToken};
use crate::schema::{Artifact, ArtifactData, SchemaKind, TypeId};
use crate::template::Template;

fn shape(at: &str, what: impl std::fmt::Display) -> BundleError {
    BundleError::Shape(format!("{at}: {what}"))
}

fn member<'v>(m: &'v Map<String, Value>, k: &str, at: &str) -> Result<&'v Value, BundleError> {
    m.get(k).ok_or_else(|| shape(at, format!("no `{k}`")))
}

fn object<'v>(v: &'v Value, at: &str) -> Result<&'v Map<String, Value>, BundleError> {
    v.as_object().ok_or_else(|| shape(at, "not an object"))
}

fn text<'v>(m: &'v Map<String, Value>, k: &str, at: &str) -> Result<&'v str, BundleError> {
    member(m, k, at)?
        .as_str()
        .ok_or_else(|| shape(at, format!("`{k}` is not a string")))
}

/// A member read through its serde type (the authoring enums, `params`,
/// `annotations`, `history`, `deprecated`).
fn de<T: DeserializeOwned>(m: &Map<String, Value>, k: &str, at: &str) -> Result<T, BundleError> {
    serde_json::from_value(member(m, k, at)?.clone()).map_err(|e| shape(at, format!("`{k}`: {e}")))
}

fn type_id(v: &Value, at: &str) -> Result<TypeId, BundleError> {
    let m = object(v, at)?;
    let name = || text(m, "name", at).map(str::to_owned);
    let schema = || text(m, "schema", at).map(str::to_owned);
    match text(m, "kind", at)? {
        "protobuf" => Ok(TypeId::Protobuf {
            name: name()?,
            schema: schema()?,
        }),
        "jsonschema" => Ok(TypeId::JsonSchema {
            name: name()?,
            schema: schema()?,
        }),
        "raw" => Ok(TypeId::Raw {
            media_type: text(m, "media_type", at)?.to_owned(),
            media_param: de(m, "media_param", at)?,
        }),
        other => Err(shape(at, format!("unknown type kind {other:?}"))),
    }
}

fn opt_type(m: &Map<String, Value>, k: &str, at: &str) -> Result<Option<TypeId>, BundleError> {
    match member(m, k, at)? {
        Value::Null => Ok(None),
        v => type_id(v, &format!("{at}.{k}")).map(Some),
    }
}

fn resource(v: &Value, i: usize) -> Result<Resource, BundleError> {
    let at = format!("contract.resources[{i}]");
    let m = object(v, &at)?;
    let template = Template::parse(text(m, "template", &at)?).map_err(|e| shape(&at, e))?;
    let kind: Kind = de(m, "kind", &at)?;
    let token = KindToken::from_str(text(m, "token", &at)?).map_err(|e| shape(&at, e))?;
    let explicit = matches!(token, KindToken::ExplicitStream | KindToken::ExplicitState);
    if token_of(kind, explicit) != token {
        return Err(shape(
            &at,
            format!("token {token} is not one of kind {:?}", kind.as_str()),
        ));
    }
    let body = if kind == Kind::Operation {
        Body::Operation(Operation {
            request: type_id(member(m, "request", &at)?, &format!("{at}.request"))?,
            response: type_id(member(m, "response", &at)?, &format!("{at}.response"))?,
            error: opt_type(m, "error", &at)?,
            summary: opt_type(m, "summary", &at)?,
            encoding: de(m, "encoding", &at)?,
            idempotent: de(m, "idempotent", &at)?,
            fanout: de(m, "fanout", &at)?,
            serving: de(m, "serving", &at)?,
            replies: de(m, "replies", &at)?,
            timeout_ms: de(m, "timeout_ms", &at)?,
            priority: de(m, "priority", &at)?,
        })
    } else {
        let rate = match member(m, "rate", &at)? {
            Value::Null => None,
            Value::String(s) => Some(
                Rate::parse(s).ok_or_else(|| shape(&at, format!("`rate` {s:?} is not a rate")))?,
            ),
            _ => return Err(shape(&at, "`rate` is not a string")),
        };
        Body::Data(Data {
            type_: type_id(member(m, "type", &at)?, &format!("{at}.type"))?,
            attachment: opt_type(m, "attachment", &at)?,
            encoding: de(m, "encoding", &at)?,
            attachment_encoding: de(m, "attachment_encoding", &at)?,
            reliability: de(m, "reliability", &at)?,
            congestion: de(m, "congestion", &at)?,
            priority: de(m, "priority", &at)?,
            express: de(m, "express", &at)?,
            history: de::<Option<HistoryParams>>(m, "history", &at)?,
            rate,
            retention_s: de(m, "retention_s", &at)?,
        })
    };
    Ok(Resource {
        template,
        kind,
        token,
        doc: None,
        params: de::<BTreeMap<String, ParamType>>(m, "params", &at)?,
        cardinality: de(m, "cardinality", &at)?,
        epoch: de(m, "epoch", &at)?,
        optional: de(m, "optional", &at)?,
        gate: de(m, "gate", &at)?,
        deprecated: de(m, "deprecated", &at)?,
        annotations: de(m, "annotations", &at)?,
        body,
    })
}

fn requirement(role: &str, v: &Value) -> Result<Requirement, BundleError> {
    let at = format!("contract.requires.{role}");
    let m = object(v, &at)?;
    Ok(Requirement {
        interface: IfaceId::from_str(text(m, "interface", &at)?).map_err(|e| shape(&at, e))?,
        resources: de(m, "resources", &at)?,
        cardinality: de(m, "cardinality", &at)?,
        optional: de(m, "optional", &at)?,
        doc: None,
        annotations: de(m, "annotations", &at)?,
    })
}

fn artifact(listed: &Value, b: &Bundle, i: usize) -> Result<(String, Artifact), BundleError> {
    let at = format!("contract.schemas[{i}]");
    let m = object(listed, &at)?;
    let id = text(m, "id", &at)?;
    let name = text(m, "name", &at)?.to_owned();
    let entry = b
        .schemas
        .get(id)
        .ok_or_else(|| BundleError::MissingSchema(id.to_owned()))?;
    let data = entry
        .get("data")
        .ok_or_else(|| shape(&format!("schemas.{id}"), "no `data`"))?;
    let (kind, data) = match text(m, "kind", &at)? {
        "protobuf" => {
            let b64 = data
                .as_str()
                .ok_or_else(|| shape(&format!("schemas.{id}"), "data is not base64"))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| shape(&format!("schemas.{id}"), e))?;
            (SchemaKind::Protobuf, ArtifactData::Protobuf(bytes))
        }
        "jsonschema" => (SchemaKind::JsonSchema, ArtifactData::Json(data.clone())),
        other => return Err(shape(&at, format!("unknown schema kind {other:?}"))),
    };
    Ok((id.to_owned(), Artifact { kind, name, data }))
}

impl Contract {
    /// The contract model inside a bundle, read from its canonical form
    /// (spec §9.5, §9.6). `minor`, `summary` and every `doc` are `None`:
    /// the canonical form excludes them. [`Bundle::build`] of the result is
    /// the bundle given, byte for byte, so a bundle built once (by
    /// `zk2 contract bundle`, or embedded by codegen) is never rebuilt
    /// differently (#611).
    ///
    /// The bundle should come from [`Bundle::verify`]: this reads its shape
    /// and refuses what is not a canonical contract
    /// ([`BundleError::Shape`]), but checks no hash.
    ///
    /// # Errors
    /// A contract whose `format` is not this version's, or whose shape is
    /// not the canonical form's.
    pub fn from_bundle(b: &Bundle) -> Result<Self, BundleError> {
        let c = object(&b.contract, "contract")?;
        let format = text(c, "format", "contract")?;
        if format != FORMAT {
            return Err(BundleError::Format(format.to_owned()));
        }
        let iface = IfaceId::from_str(text(c, "interface", "contract")?)
            .map_err(|e| shape("contract", e))?;
        let uses = de::<Vec<String>>(c, "uses", "contract")?
            .iter()
            .map(|u| IfaceId::from_str(u).map_err(|e| shape("contract.uses", e)))
            .collect::<Result<Vec<_>, _>>()?;
        let resources = member(c, "resources", "contract")?
            .as_array()
            .ok_or_else(|| shape("contract.resources", "not a list"))?
            .iter()
            .enumerate()
            .map(|(i, r)| resource(r, i))
            .collect::<Result<Vec<_>, _>>()?;
        let requires = object(member(c, "requires", "contract")?, "contract.requires")?
            .iter()
            .map(|(role, r)| Ok((role.clone(), requirement(role, r)?)))
            .collect::<Result<BTreeMap<_, _>, BundleError>>()?;
        let artifacts = member(c, "schemas", "contract")?
            .as_array()
            .ok_or_else(|| shape("contract.schemas", "not a list"))?
            .iter()
            .enumerate()
            .map(|(i, s)| artifact(s, b, i))
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        Ok(Self {
            iface,
            minor: None,
            summary: None,
            uses,
            resources,
            requires,
            artifacts,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::bundle::Bundle;
    use crate::contract::{Contract, load_path};

    fn tomls(dir: &Path, prefix: &str, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if p.is_dir() && !name.starts_with('.') {
                tomls(&p, prefix, out);
            } else if name.starts_with(prefix)
                && name.ends_with(".toml")
                && !name.ends_with(".bindings.toml")
                && !name.ends_with(".enrollment.toml")
            {
                out.push(p);
            }
        }
    }

    /// The round trip is the test: for every valid contract of the
    /// conformance set and the examples, the model read back from its
    /// bundle builds the same bundle, byte for byte.
    #[test]
    fn a_bundle_round_trips_through_its_model() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut files = Vec::new();
        tomls(&root.join("spec/conformance/contracts"), "ok-", &mut files);
        let ok = files.len();
        tomls(&root.join("examples/zk2"), "", &mut files);
        assert!(ok >= 10 && files.len() >= ok + 24, "{} files", files.len());
        for p in files {
            let l = load_path(&p);
            let c = l
                .contract
                .unwrap_or_else(|| panic!("{}: {}", p.display(), l.report));
            let bytes = Bundle::build(&c).to_bytes();
            let verified = Bundle::verify(&bytes).unwrap();
            let back =
                Contract::from_bundle(&verified).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            assert_eq!(
                Bundle::build(&back).to_bytes(),
                bytes,
                "{}: the round trip changed the bundle",
                p.display()
            );
            assert_eq!(back.iface, c.iface);
            assert_eq!(back.resources.len(), c.resources.len());
        }
    }

    #[test]
    fn a_bundle_that_is_not_canonical_is_refused() {
        let src = "[interface]\nname = \"t\"\nmajor = 1\nminor = 0\n\
                   [resources.a]\nkind = \"stream\"\ntype = { raw = \"text/plain\" }\n";
        let c = crate::contract::load_str(src, Path::new("."), None)
            .contract
            .unwrap();
        let b = Bundle::build(&c);
        let mut wrong = b.clone();
        wrong.contract["resources"][0]["token"] = "@op".into();
        assert_eq!(Contract::from_bundle(&wrong).unwrap_err().tag(), "shape");
        let mut wrong = b.clone();
        wrong.contract["resources"][0]
            .as_object_mut()
            .unwrap()
            .remove("priority");
        assert_eq!(Contract::from_bundle(&wrong).unwrap_err().tag(), "shape");
        let mut wrong = b;
        wrong.contract["format"] = "zk2-contract/draft-0".into();
        assert_eq!(Contract::from_bundle(&wrong).unwrap_err().tag(), "format");
    }
}

//! The canonical contract and its fingerprint (r3 §3.11).
//!
//! - **The canonical form is fully explicit JSON:** every field of every
//!   resource is present, `null` when absent, defaults expanded (D2).
//!   Documentation (`doc`, `summary`) and `minor` are excluded.
//! - **Resources are a list sorted by (kind token, template).** Gates and
//!   requirement resources are sorted sets; objects are ordered by JCS.
//! - **`schemas` lists every artifact the contract carries**, so a change
//!   to any schema file, including one that is only `$ref`'d, changes the
//!   fingerprint (over-detect, never under-detect).
//! - **Restrictions (spec §9.5):** every string printable ASCII (E027),
//!   every integer within ±(2^53−1), every float finite and, when JCS writes
//!   it as an integer, within the same range (E028). On that domain every
//!   JCS implementation agrees.
//! - **The fingerprint** is `sha256:` + hex of the sha256 of the JCS bytes.

use std::fmt;

use serde_json::{Map, Value, json};

use crate::contract::{Body, Contract, Requirement, Resource};
use crate::diag::{Diagnostic, Report};
use crate::grammar::{KeyError, Sha256Hex};
use crate::schema::{TypeId, jcs, sha256_id};

/// The format tag inside every canonical contract. It changes when the
/// canonical form does, so a format revision is visible in the fingerprint.
pub const FORMAT: &str = "zk2-contract/draft-1";

/// The largest integer the canonical form admits: 2^53 − 1.
pub const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

fn ser<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("authoring enums serialize")
}

fn type_value(t: &TypeId) -> Value {
    match t {
        TypeId::Protobuf { name, schema } => {
            json!({"kind": "protobuf", "name": name, "schema": schema})
        }
        TypeId::JsonSchema { name, schema } => {
            json!({"kind": "jsonschema", "name": name, "schema": schema})
        }
        TypeId::Raw {
            media_type,
            media_param,
        } => {
            json!({"kind": "raw", "media_type": media_type, "media_param": media_param})
        }
    }
}

fn opt_type(t: Option<&TypeId>) -> Value {
    t.map_or(Value::Null, type_value)
}

fn resource_value(r: &Resource) -> Value {
    let mut m = Map::new();
    m.insert("token".into(), json!(r.token.as_str()));
    m.insert("template".into(), json!(r.template.as_str()));
    m.insert("kind".into(), json!(r.kind.as_str()));
    m.insert("params".into(), ser(&r.params));
    m.insert("cardinality".into(), json!(r.cardinality));
    m.insert("epoch".into(), json!(r.epoch));
    m.insert("optional".into(), json!(r.optional));
    m.insert("gate".into(), json!(r.gate));
    m.insert(
        "deprecated".into(),
        r.deprecated.as_ref().map_or(
            Value::Null,
            |d| json!({"since": d.since, "replaced_by": d.replaced_by, "reason": d.reason}),
        ),
    );
    m.insert("annotations".into(), ser(&r.annotations));
    match &r.body {
        Body::Data(d) => {
            m.insert("type".into(), type_value(&d.type_));
            m.insert("attachment".into(), opt_type(d.attachment.as_ref()));
            m.insert("encoding".into(), ser(&d.encoding));
            m.insert("attachment_encoding".into(), ser(&d.attachment_encoding));
            m.insert("reliability".into(), ser(&d.reliability));
            m.insert("congestion".into(), ser(&d.congestion));
            m.insert("priority".into(), ser(&d.priority));
            m.insert("express".into(), json!(d.express));
            m.insert(
                "history".into(),
                d.history.as_ref().map_or(
                    Value::Null,
                    |h| json!({"depth": h.depth, "miss_detection_ms": h.miss_detection_ms}),
                ),
            );
            m.insert("rate".into(), json!(d.rate.map(|r| r.to_string())));
            m.insert("retention_s".into(), json!(d.retention_s));
        }
        Body::Operation(o) => {
            m.insert("request".into(), type_value(&o.request));
            m.insert("response".into(), type_value(&o.response));
            m.insert("error".into(), opt_type(o.error.as_ref()));
            m.insert("summary".into(), opt_type(o.summary.as_ref()));
            m.insert("encoding".into(), ser(&o.encoding));
            m.insert("idempotent".into(), json!(o.idempotent));
            m.insert("fanout".into(), ser(&o.fanout));
            m.insert("serving".into(), ser(&o.serving));
            m.insert("replies".into(), ser(&o.replies));
            m.insert("timeout_ms".into(), json!(o.timeout_ms));
            m.insert("priority".into(), ser(&o.priority));
        }
    }
    Value::Object(m)
}

fn requirement_value(r: &Requirement) -> Value {
    json!({
        "interface": r.interface.to_string(),
        "resources": r.resources,
        "cardinality": ser(&r.cardinality),
        "optional": r.optional,
        "annotations": ser(&r.annotations),
    })
}

/// The canonical contract, as a JSON value.
#[must_use]
pub fn canonical(c: &Contract) -> Value {
    let schemas: Vec<Value> = c
        .artifacts
        .iter()
        .map(|(id, a)| json!({"id": id, "kind": a.kind.as_str(), "name": a.name}))
        .collect();
    let requires: Map<String, Value> = c
        .requires
        .iter()
        .map(|(k, r)| (k.clone(), requirement_value(r)))
        .collect();
    json!({
        "format": FORMAT,
        "interface": c.iface.to_string(),
        "uses": c.uses.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "schemas": schemas,
        "resources": c.resources.iter().map(resource_value).collect::<Vec<_>>(),
        "requires": requires,
    })
}

/// The canonical bytes: JCS of [`canonical`].
#[must_use]
pub fn canonical_bytes(c: &Contract) -> Vec<u8> {
    jcs(&canonical(c))
}

/// Whether a JSON number is outside the canonical form's domain: an
/// integer beyond ±(2^53−1), or a float that JCS writes as such an integer
/// (an integral value below 10^21, where ECMAScript switches to exponent
/// form). `1e16` is the integer `10000000000000000` once serialized, so a
/// reader of the bytes sees an integer beyond the range (spec §9.5).
#[must_use]
pub fn outside_safe_range(n: &serde_json::Number) -> bool {
    #[allow(clippy::cast_precision_loss)] // 2^53 − 1 is exact in an f64
    let max = MAX_SAFE_INTEGER as f64;
    match (n.as_u64(), n.as_i64(), n.as_f64()) {
        (Some(u), _, _) => u > MAX_SAFE_INTEGER,
        (None, Some(i), _) => i.unsigned_abs() > MAX_SAFE_INTEGER,
        (None, None, Some(f)) => f.fract() == 0.0 && f.abs() > max && f.abs() < 1e21,
        _ => false,
    }
}

/// Reports every string outside printable ASCII (E027) and every number
/// outside the canonical form's domain (E028, [`outside_safe_range`]), with
/// its JSON path. Bundle verification runs this on the contract it reads.
pub fn check_restrictions(v: &Value, report: &mut Report) {
    walk_restrictions(v, false, report);
}

/// [`check_restrictions`] on a contract being linted, which can also hold a
/// float that is not finite: an annotation's `nan` or `inf`, carried as a
/// [`crate::authoring::NON_FINITE`] marker. JSON has no such number (E028).
pub(crate) fn check_contract_restrictions(v: &Value, report: &mut Report) {
    walk_restrictions(v, true, report);
}

fn walk_restrictions(v: &Value, lint: bool, report: &mut Report) {
    fn ascii(s: &str) -> bool {
        s.bytes().all(|b| (0x20..=0x7e).contains(&b))
    }
    fn go(v: &Value, lint: bool, path: &mut String, report: &mut Report) {
        match v {
            Value::String(s) if !ascii(s) => report.push(Diagnostic::error(
                "E027",
                format!("canonical{path}"),
                format!("{s:?} is not printable ASCII"),
            )),
            Value::Number(n) => {
                if outside_safe_range(n) {
                    report.push(Diagnostic::error(
                        "E028",
                        format!("canonical{path}"),
                        format!("{n} is, or serializes as, an integer outside ±(2^53−1)"),
                    ));
                }
            }
            Value::Array(a) => {
                for (i, x) in a.iter().enumerate() {
                    let len = path.len();
                    path.push_str(&format!("[{i}]"));
                    go(x, lint, path, report);
                    path.truncate(len);
                }
            }
            Value::Object(m)
                if lint && m.len() == 1 && m.contains_key(crate::authoring::NON_FINITE) =>
            {
                report.push(Diagnostic::error(
                    "E028",
                    format!("canonical{path}"),
                    format!(
                        "{} is not a finite number; JSON has none",
                        m[crate::authoring::NON_FINITE]
                    ),
                ));
            }
            Value::Object(m) => {
                for (k, x) in m {
                    let len = path.len();
                    if !ascii(k) {
                        report.push(Diagnostic::error(
                            "E027",
                            format!("canonical{path}"),
                            format!("key {k:?} is not printable ASCII"),
                        ));
                    }
                    path.push_str(&format!(".{k}"));
                    go(x, lint, path, report);
                    path.truncate(len);
                }
            }
            _ => {}
        }
    }
    go(v, lint, &mut String::new(), report);
}

/// A contract fingerprint: the sha256 of its canonical bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint(Sha256Hex);

impl Fingerprint {
    /// Of canonical bytes.
    #[must_use]
    pub fn of_bytes(canonical: &[u8]) -> Self {
        let id = sha256_id(canonical);
        Self(Sha256Hex::new(&id["sha256:".len()..]).expect("sha256_id is 64 lowercase hex"))
    }

    /// Of a contract.
    #[must_use]
    pub fn of(c: &Contract) -> Self {
        Self::of_bytes(&canonical_bytes(c))
    }

    /// Parses `sha256:<64 lowercase hex>`.
    pub fn parse(s: &str) -> Result<Self, KeyError> {
        let hex = s.strip_prefix("sha256:").unwrap_or("");
        Sha256Hex::new(hex).map(Self)
    }

    /// The 64-hex digest, as the contract key spells it.
    #[must_use]
    pub fn hex(&self) -> &Sha256Hex {
        &self.0
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::contract::load_str;

    const SPELLED: &str = r#"
[interface]
name = "t"
major = 1
minor = 3
summary = "docs do not count"

[resources."a"]
kind = "stream"
doc = "neither does this"
type = { raw = "text/plain" }
reliability = "best_effort"
congestion = "drop"
priority = "data"
express = false
history = false
"#;

    const DEFAULTED: &str = r#"
[interface]
name = "t"
major = 1
minor = 4

[resources."a"]
kind = "stream"
type = { raw = "text/plain" }
"#;

    #[test]
    fn defaulted_and_spelled_out_fingerprint_alike() {
        let a = load_str(SPELLED, Path::new("."), None).contract.unwrap();
        let b = load_str(DEFAULTED, Path::new("."), None).contract.unwrap();
        assert_eq!(canonical_bytes(&a), canonical_bytes(&b));
        assert_eq!(Fingerprint::of(&a), Fingerprint::of(&b));
        let fp = Fingerprint::of(&a).to_string();
        assert_eq!(Fingerprint::parse(&fp).unwrap(), Fingerprint::of(&a));
    }

    #[test]
    fn a_qos_change_changes_the_fingerprint() {
        let a = load_str(DEFAULTED, Path::new("."), None).contract.unwrap();
        let src = DEFAULTED.replace("text/plain\" }", "text/plain\" }\npriority = \"data_high\"");
        let b = load_str(&src, Path::new("."), None).contract.unwrap();
        assert_ne!(Fingerprint::of(&a), Fingerprint::of(&b));
    }

    #[test]
    fn restrictions() {
        let mut r = Report::default();
        check_restrictions(
            &json!({"a": "é", "b": [9_007_199_254_740_992_u64], "c": -9_007_199_254_740_991_i64}),
            &mut r,
        );
        assert_eq!(r.codes(), ["E027", "E028"]);
        let src = DEFAULTED.replace(
            "[resources",
            "[defaults]\nannotations = { \"x.y\" = 1 }\n[resources",
        );
        assert_eq!(
            load_str(&src, Path::new("."), None).report.codes(),
            ["E020"]
        );
    }

    /// Spec §9.5: a float is in the domain when it is finite and, if JCS
    /// writes it as an integer (integral, below 10^21), within ±(2^53−1).
    #[test]
    fn floats_outside_the_domain() {
        let codes = |v: &str| {
            let src = format!(
                "[interface]\nname = \"t\"\nmajor = 1\nminor = 0\nuses = [\"freshness.v1\"]\n\
                 [resources.a]\nkind = \"stream\"\ntype = {{ raw = \"text/plain\" }}\n\
                 annotations = {{ \"freshness.ttl_s\" = {v} }}\n"
            );
            load_str(&src, Path::new("."), None).report.codes()
        };
        for bad in [
            "nan",
            "+inf",
            "-inf",
            "1e16",
            "-1e16",
            "9007199254740992.0",
            "[1, nan]",
        ] {
            assert_eq!(codes(bad), ["E028"], "{bad}");
        }
        for ok in [
            "1.0",
            "1.5",
            "9007199254740991.0",
            "1e21",
            "1e300",
            "-0.0",
            "0.1",
        ] {
            assert!(codes(ok).is_empty(), "{ok}");
        }
        // `1.0` and `1` have the same canonical bytes.
        let one = |v: &str| {
            let src = format!(
                "[interface]\nname = \"t\"\nmajor = 1\nminor = 0\nuses = [\"freshness.v1\"]\n\
                 [resources.a]\nkind = \"stream\"\ntype = {{ raw = \"text/plain\" }}\n\
                 annotations = {{ \"freshness.ttl_s\" = {v} }}\n"
            );
            canonical_bytes(&load_str(&src, Path::new("."), None).contract.unwrap())
        };
        assert_eq!(one("1.0"), one("1"));
        // Bundle verification reads bytes, where no float is ever `nan`; an
        // integral float written with a fraction is still out of range.
        let mut r = Report::default();
        check_restrictions(&json!({"a": 1e16, "b": 1.5, "c": 1e21}), &mut r);
        assert_eq!(r.codes(), ["E028"]);
    }
}

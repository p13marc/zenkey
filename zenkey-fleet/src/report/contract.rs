//! zk2 contracts as a tool shows them (spec §9), and what asking for one
//! found (§8.4): `iface show`'s revisions and `schema show`.
//!
//! A contract revision is identified by its fingerprint and read from its
//! bundle, which a tool retrieves from the bus or loads from a `.history`
//! root or the authoring files. The view below is the canonical contract's
//! content, typed for rendering: the spec's own enums (`kind`, `priority`,
//! `fanout`, …) serialize in their spec spelling, and a type reference is
//! its kind and name. Documentation (`doc`, `summary`, `minor`) is not in a
//! bundle (§9.5), so it is present only for a contract loaded from its
//! authoring file — absent, never empty.
//!
//! `compat` (#612, FJ4) is here too: the classifier's verdict on two
//! revisions (§9.8), each side named by where it was read from.

use std::collections::BTreeMap;

use serde::Serialize;
use zenkey_model::authoring::{
    Congestion, Deprecated, Encoding, Fanout, HistoryParams, Kind, ParamType, Priority,
    Reliability, Replies, RequireCardinality, Serving,
};

use crate::report::{Asked, Judgement};

/// One contract revision.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContractView {
    /// `<name>.v<major>`.
    pub iface: String,
    /// `sha256:` + 64 lowercase hex digits.
    pub fingerprint: String,
    /// Informative, and not in the bundle: present for an authoring file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minor: Option<u32>,
    /// Documentation, and not in the bundle: present for an authoring file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The profiles the contract uses, `<name>.v<major>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<String>,
    /// Sorted by (kind token, template), as the canonical form sorts them.
    pub resources: Vec<ResourceView>,
    /// The roles the contract declares (§3.1), by role.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<RequirementView>,
    /// Every schema artifact the bundle carries, by id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schemas: Vec<SchemaArtifact>,
}

/// One resource, every default expanded (§9.3).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResourceView {
    /// `<kind token>/<template>`: the descriptor's naming (§3.3).
    pub name: String,
    pub kind: Kind,
    /// The kind token at key position 5 (`stream`, `@stream`, `@op`, …).
    pub token: String,
    pub template: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, ParamType>,
    pub optional: bool,
    /// The gates, an AND (`capability:<name>`, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gate: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<u64>,
    /// The template parameter that carries a member's epoch (r3.3 D9b).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<Deprecated>,
    /// Documentation, and not in the bundle: present for an authoring file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// What the resource carries and how: a data body or an operation's.
    #[serde(flatten)]
    pub body: ResourceBody,
}

/// A resource's body. Untagged and flattened: `kind` above already says
/// which one it is.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ResourceBody {
    /// A stream, state or event.
    Data {
        #[serde(rename = "type")]
        type_: TypeView,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attachment: Option<TypeView>,
        /// Set exactly when the type is a JSON Schema type.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encoding: Option<Encoding>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attachment_encoding: Option<Encoding>,
        reliability: Reliability,
        congestion: Congestion,
        priority: Priority,
        express: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        history: Option<HistoryParams>,
        /// Events only: `rare`, `low`, or `<n>/<unit>`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rate: Option<String>,
        /// Events only: how far back a replay GET may reach.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retention_s: Option<u64>,
    },
    /// An operation (§5).
    Operation {
        request: TypeView,
        response: TypeView,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<TypeView>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<TypeView>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encoding: Option<Encoding>,
        idempotent: bool,
        fanout: Fanout,
        serving: Serving,
        replies: Replies,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
        priority: Priority,
    },
}

/// A resolved type reference (§7.1): its kind (`protobuf`, `jsonschema`,
/// `raw`) and its name — the message's full name, the JSON Schema
/// definition's, or the raw media type (with `;<param>` when the contract
/// names one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TypeView {
    pub kind: String,
    pub name: String,
    /// The artifact that defines it (`sha256:…`); absent for a raw type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
}

/// One role a contract declares (§3.1).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RequirementView {
    pub role: String,
    pub interface: String,
    pub cardinality: RequireCardinality,
    pub optional: bool,
    /// The resources of `interface` the role reads; absent means all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<Vec<String>>,
}

/// One schema artifact a bundle carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SchemaArtifact {
    /// `sha256:…` over the artifact.
    pub id: String,
    /// `protobuf` or `jsonschema`.
    pub kind: String,
    /// The protobuf file name or the JSON Schema file stem.
    pub name: String,
}

/// What asking for one revision found (§8.4).
///
/// Not asked is not one of these: a field holding this is an
/// [`Asked`](crate::report::Asked), and absent when the revision was never
/// looked for.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum ContractAnswer {
    /// In hand and verified against its fingerprint.
    Held {
        source: ContractSource,
        contract: Box<ContractView>,
    },
    /// No reply verified, after the retry with target `All` (§8.4). Each
    /// refused reply's verification tag, in arrival order (§9.6); empty
    /// when nothing answered at all.
    Unavailable { refused: Vec<String> },
    /// A bundle that verified, whose contract this build cannot read: a
    /// canonical form of another format, most likely.
    Unreadable { reason: String },
}

/// Where a revision in hand came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractSource {
    /// Retrieved from a holder on the bus (§8.4).
    Bus,
    /// Read from a `.history` root (§9.7).
    History,
    /// Built from an authoring file.
    File,
    /// Read from a bundle file named on its own (`*.bundle.json`, §9.6).
    Bundle,
}

/// `schema show`: the schema artifacts one revision's bundle carries, and
/// which resource member names which type in them (§7.1, §9.5).
///
/// The bundle is the whole source: a tool that was never compiled against
/// the contract reads its types from here, so this is what it can know.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SchemaView {
    /// `<name>.v<major>`.
    pub iface: String,
    /// `sha256:` + 64 lowercase hex digits.
    pub fingerprint: String,
    pub source: ContractSource,
    /// The resource asked about, `<kind token>/<template>`; absent when the
    /// whole revision was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    /// Every member's type, by resource then member. A raw type names no
    /// artifact: the contract says its bytes are not a tool's to read.
    pub members: Vec<SchemaMember>,
    /// The artifacts those types live in — every artifact, for the whole
    /// revision — by id.
    pub artifacts: Vec<SchemaDocument>,
}

/// One member of one resource, and the type it names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SchemaMember {
    /// `<kind token>/<template>`.
    pub resource: String,
    /// `type`, `attachment`, `request`, `response`, `error` or `summary`.
    pub member: String,
    #[serde(rename = "type")]
    pub type_: TypeView,
}

/// One schema artifact, and its document when it was asked for.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SchemaDocument {
    /// `sha256:…` over the artifact.
    pub id: String,
    /// `protobuf` or `jsonschema`.
    pub kind: String,
    /// The protobuf file name or the JSON Schema file stem.
    pub name: String,
    /// The JSON Schema document as the bundle carries it, or the protobuf
    /// descriptor set read as its files, messages and enums (the bundle
    /// carries the set's bytes, which are not a document anyone reads).
    /// Absent when not asked for.
    #[serde(default, skip_serializing_if = "Asked::is_not_asked")]
    pub document: Asked<serde_json::Value>,
}

/// How a change is judged (§9.8): the classifier's three classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatClass {
    Compatible,
    /// A human accepts it.
    Review,
    /// CI refuses it.
    Breaking,
}

impl CompatClass {
    /// The class as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            CompatClass::Compatible => "compatible",
            CompatClass::Review => "review",
            CompatClass::Breaking => "breaking",
        }
    }
}

/// `compat <old> <new>`: the compatibility classifier's verdict on one
/// change (§9.8), both directions, with every review and breaking finding.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompatReport {
    pub old: CompatSide,
    pub new: CompatSide,
    /// The worst class found; `compatible` when there is no finding.
    pub class: CompatClass,
    /// Every review and breaking change.
    pub findings: Vec<CompatFinding>,
    /// Warnings that do not change the class (`field_deleted_unreserved`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<CompatFinding>,
}

impl CompatReport {
    /// The verdict on the RFC 13 core. The judged claim is the finding, so
    /// a review or breaking change is `Established` and a compatible one
    /// `NotEstablished`: exit 1 and exit 0, through the one projection.
    pub fn to_judgement(&self) -> Judgement {
        match self.class {
            CompatClass::Compatible => Judgement::NotEstablished {
                reason: "compatible: no review or breaking change".into(),
            },
            CompatClass::Review | CompatClass::Breaking => Judgement::Established,
        }
    }
}

/// One side of a comparison: what was named, and the revision it resolved
/// to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompatSide {
    /// As given: a path, or `<iface>[@<fingerprint>]`.
    pub input: String,
    pub source: ContractSource,
    /// The interface the revision declares, `<name>.v<major>`.
    pub iface: String,
    pub fingerprint: String,
}

/// One judged change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompatFinding {
    pub class: CompatClass,
    /// The classifier's stable rule name (`field_renamed`, …).
    pub rule: String,
    /// Where: `resources."<template>"`, a field path, a role.
    pub at: String,
    pub detail: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stream() -> ResourceView {
        ResourceView {
            name: "stream/bandwidth/{ns}/{iface}".into(),
            kind: Kind::Stream,
            token: "stream".into(),
            template: "bandwidth/{ns}/{iface}".into(),
            params: [
                ("iface".to_owned(), ParamType::String),
                ("ns".to_owned(), ParamType::String),
            ]
            .into(),
            optional: false,
            gate: vec![],
            cardinality: Some(1024),
            epoch: None,
            deprecated: None,
            doc: None,
            body: ResourceBody::Data {
                type_: TypeView {
                    kind: "jsonschema".into(),
                    name: "BandwidthUpdate".into(),
                    schema: Some("sha256:00".into()),
                },
                attachment: None,
                encoding: Some(Encoding::Json),
                attachment_encoding: None,
                reliability: Reliability::BestEffort,
                congestion: Congestion::Drop,
                priority: Priority::Data,
                express: false,
                history: None,
                rate: None,
                retention_s: None,
            },
        }
    }

    fn op() -> ResourceView {
        ResourceView {
            name: "@op/diagnostics".into(),
            kind: Kind::Operation,
            token: "@op".into(),
            template: "diagnostics".into(),
            params: BTreeMap::new(),
            optional: true,
            gate: vec!["capability:diag".into()],
            cardinality: None,
            epoch: None,
            deprecated: None,
            doc: Some("Run diagnostics.".into()),
            body: ResourceBody::Operation {
                request: TypeView {
                    kind: "jsonschema".into(),
                    name: "DiagnosticsRequest".into(),
                    schema: Some("sha256:00".into()),
                },
                response: TypeView {
                    kind: "raw".into(),
                    name: "application/octet-stream".into(),
                    schema: None,
                },
                error: None,
                summary: None,
                encoding: Some(Encoding::Cbor),
                idempotent: true,
                fanout: Fanout::Allowed,
                serving: Serving::Exclusive,
                replies: Replies::One,
                timeout_ms: Some(500),
                priority: Priority::InteractiveHigh,
            },
        }
    }

    /// `iface show`'s revision, held: the body flattens beside `kind`, the
    /// spec's enums keep the spec's spelling, and documentation is absent
    /// rather than empty when the bundle had none.
    #[test]
    fn contract_answer_held_json_shape_is_pinned() {
        let answer = ContractAnswer::Held {
            source: ContractSource::Bus,
            contract: Box::new(ContractView {
                iface: "tc.netif.v1".into(),
                fingerprint: format!("sha256:{}", "ab".repeat(32)),
                minor: None,
                summary: None,
                uses: vec!["freshness.v1".into()],
                resources: vec![stream(), op()],
                requires: vec![RequirementView {
                    role: "clock".into(),
                    interface: "time.v1".into(),
                    cardinality: RequireCardinality::One,
                    optional: true,
                    resources: None,
                }],
                schemas: vec![SchemaArtifact {
                    id: "sha256:00".into(),
                    kind: "jsonschema".into(),
                    name: "tc".into(),
                }],
            }),
        };
        assert_eq!(
            serde_json::to_value(&answer).expect("serialize"),
            json!({
                "answer": "held",
                "source": "bus",
                "contract": {
                    "iface": "tc.netif.v1",
                    "fingerprint": format!("sha256:{}", "ab".repeat(32)),
                    "uses": ["freshness.v1"],
                    "resources": [
                        {
                            "name": "stream/bandwidth/{ns}/{iface}",
                            "kind": "stream",
                            "token": "stream",
                            "template": "bandwidth/{ns}/{iface}",
                            "params": {"iface": "string", "ns": "string"},
                            "optional": false,
                            "cardinality": 1024,
                            "type": {"kind": "jsonschema", "name": "BandwidthUpdate", "schema": "sha256:00"},
                            "encoding": "json",
                            "reliability": "best_effort",
                            "congestion": "drop",
                            "priority": "data",
                            "express": false,
                        },
                        {
                            "name": "@op/diagnostics",
                            "kind": "operation",
                            "token": "@op",
                            "template": "diagnostics",
                            "optional": true,
                            "gate": ["capability:diag"],
                            "doc": "Run diagnostics.",
                            "request": {"kind": "jsonschema", "name": "DiagnosticsRequest", "schema": "sha256:00"},
                            "response": {"kind": "raw", "name": "application/octet-stream"},
                            "encoding": "cbor",
                            "idempotent": true,
                            "fanout": "allowed",
                            "serving": "exclusive",
                            "replies": "one",
                            "timeout_ms": 500,
                            "priority": "interactive_high",
                        },
                    ],
                    "requires": [{
                        "role": "clock",
                        "interface": "time.v1",
                        "cardinality": "one",
                        "optional": true,
                    }],
                    "schemas": [{"id": "sha256:00", "kind": "jsonschema", "name": "tc"}],
                },
            })
        );
    }

    /// The document `schema show --format json` prints: a member names its
    /// type, a raw type names no artifact, and an artifact's document is
    /// absent unless it was asked for.
    #[test]
    fn schema_view_json_shape_is_pinned() {
        let view = SchemaView {
            iface: "camera.v1".into(),
            fingerprint: format!("sha256:{}", "ab".repeat(32)),
            source: ContractSource::History,
            resource: Some("@stream/image".into()),
            members: vec![
                SchemaMember {
                    resource: "@stream/image".into(),
                    member: "type".into(),
                    type_: TypeView {
                        kind: "raw".into(),
                        name: "image/jpeg".into(),
                        schema: None,
                    },
                },
                SchemaMember {
                    resource: "@stream/image".into(),
                    member: "attachment".into(),
                    type_: TypeView {
                        kind: "protobuf".into(),
                        name: "camera.v1.FrameMeta".into(),
                        schema: Some("sha256:01".into()),
                    },
                },
            ],
            artifacts: vec![
                SchemaDocument {
                    id: "sha256:01".into(),
                    kind: "protobuf".into(),
                    name: "camera/v1/camera.proto".into(),
                    document: Asked::Asked(json!({"files": []})),
                },
                SchemaDocument {
                    id: "sha256:02".into(),
                    kind: "jsonschema".into(),
                    name: "info".into(),
                    document: Asked::NotAsked,
                },
            ],
        };
        assert_eq!(
            serde_json::to_value(&view).expect("serialize"),
            json!({
                "iface": "camera.v1",
                "fingerprint": format!("sha256:{}", "ab".repeat(32)),
                "source": "history",
                "resource": "@stream/image",
                "members": [
                    {"resource": "@stream/image", "member": "type", "type": {"kind": "raw", "name": "image/jpeg"}},
                    {
                        "resource": "@stream/image",
                        "member": "attachment",
                        "type": {"kind": "protobuf", "name": "camera.v1.FrameMeta", "schema": "sha256:01"},
                    },
                ],
                "artifacts": [
                    {"id": "sha256:01", "kind": "protobuf", "name": "camera/v1/camera.proto", "document": {"files": []}},
                    {"id": "sha256:02", "kind": "jsonschema", "name": "info"},
                ],
            })
        );
    }

    /// The document `compat --format json` prints, and its verdict: a
    /// finding is the claim, so review and breaking are established.
    #[test]
    fn compat_report_json_shape_is_pinned() {
        let side = |input: &str, source, fp: &str| CompatSide {
            input: input.into(),
            source,
            iface: "tc.netif.v1".into(),
            fingerprint: format!("sha256:{}", fp.repeat(32)),
        };
        let report = CompatReport {
            old: side("tc.netif.v1@abab", ContractSource::Bus, "ab"),
            new: side("tc.netif.v1.toml", ContractSource::File, "cd"),
            class: CompatClass::Breaking,
            findings: vec![CompatFinding {
                class: CompatClass::Breaking,
                rule: "resource_removed".into(),
                at: "resources.namespaces".into(),
                detail: "removed".into(),
            }],
            warnings: vec![],
        };
        assert_eq!(
            serde_json::to_value(&report).expect("serialize"),
            json!({
                "old": {
                    "input": "tc.netif.v1@abab",
                    "source": "bus",
                    "iface": "tc.netif.v1",
                    "fingerprint": format!("sha256:{}", "ab".repeat(32)),
                },
                "new": {
                    "input": "tc.netif.v1.toml",
                    "source": "file",
                    "iface": "tc.netif.v1",
                    "fingerprint": format!("sha256:{}", "cd".repeat(32)),
                },
                "class": "breaking",
                "findings": [{
                    "class": "breaking",
                    "rule": "resource_removed",
                    "at": "resources.namespaces",
                    "detail": "removed",
                }],
            })
        );
        assert_eq!(report.to_judgement(), Judgement::Established);
        let clean = CompatReport {
            class: CompatClass::Compatible,
            findings: vec![],
            ..report
        };
        assert!(matches!(
            clean.to_judgement(),
            Judgement::NotEstablished { .. }
        ));
        assert_eq!(
            serde_json::to_value(ContractSource::Bundle).expect("serialize"),
            json!("bundle")
        );
    }

    /// The two answers that are not a contract carry what was found
    /// instead: the refusal tags, or why a verified bundle did not read.
    #[test]
    fn contract_answers_without_a_contract_json_shape_is_pinned() {
        assert_eq!(
            serde_json::to_value(ContractAnswer::Unavailable {
                refused: vec!["fingerprint".into(), "error_reply".into()]
            })
            .expect("serialize"),
            json!({"answer": "unavailable", "refused": ["fingerprint", "error_reply"]})
        );
        assert_eq!(
            serde_json::to_value(ContractAnswer::Unreadable {
                reason: "format".into()
            })
            .expect("serialize"),
            json!({"answer": "unreadable", "reason": "format"})
        );
    }
}

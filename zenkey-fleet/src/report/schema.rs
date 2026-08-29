//! The schema plane (RFC 08 §7): what a producer serves for a type name,
//! and the drift between what was served and what was declared.

use super::asked::Asked;
use serde::Serialize;

/// One type's schema entry as one producer serves it (issue #51).
#[derive(Debug, Clone, Serialize)]
pub struct SchemaRow {
    pub producer: String,
    pub type_name: String,
    pub kind: String,
    pub hash: String,
    /// The schema document, when the caller asked for the full form.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<serde_json::Value>,
}

/// One producer's served `describe` reply, rendered (issue #51).
///
/// `served = false` is the honest degradation RFC 08 §7 leaves room for —
/// `describe` is a SHOULD, so a producer that serves none has said nothing
/// about its types, which is not the same as having no types.
#[derive(Debug, Clone, Serialize)]
pub struct SchemaDump {
    pub producer: String,
    pub served: bool,
    /// The declaring app, as the served set names it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    pub types: Vec<SchemaRow>,
    /// Registry-declared type names this producer's set does **not** cover —
    /// RFC 08 §7's totality clause, checked where the user is already looking.
    /// `NotAsked` = no registry was loaded, so totality was never checked —
    /// not asked is not answered no (RFC 09 §5.1 O4); `Asked(vec![])` is the
    /// actual clean bill.
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub missing: Asked<Vec<String>>,
}

/// One producer's identity claim for a type name, attributed to the host that
/// made it (#398).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SchemaServer {
    pub producer: String,
    /// The origin that served this claim — the `h-…` host id, or a verbatim
    /// service origin (#398).
    ///
    /// `describe` fans in across every host running the producer, so the
    /// producer alone does not name a claimant. Without this a mid-rollout
    /// fleet reported that a type had two identities and gave no host to go
    /// and look at — the finding you can do least with. `"?"` when the reply
    /// key did not parse under the base, the same lossy-but-stated convention
    /// [`FleetAnswer::origin`](crate::FleetAnswer::origin) uses.
    pub origin: String,
    /// The `sha256:` identity this producer served, if it served one.
    ///
    /// `NotAsked` means the describe reply carried **no** hash — which is not
    /// an empty hash, and is the distinction the flat `(String, String)` shape
    /// could not make: two producers that each said nothing compared equal and
    /// were reported as agreeing (#370).
    #[serde(skip_serializing_if = "Asked::is_not_asked")]
    pub hash: Asked<String>,
}

/// What comparing a type name's identity claims established.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftVerdict {
    /// Two or more producers served *different* identities. A defect —
    /// RFC 08 §7 calls it a `doctor` finding in as many words.
    Disagree,
    /// At least one producer served no identity at all, so agreement cannot
    /// be established. **Not a defect**: an unanswered question, and reporting
    /// it as agreement was the O4 failure (RFC 09 §5.1) this exists to name.
    Unjudgeable,
}

/// One type name's identity claims across the fleet — "a `doctor` finding" by
/// RFC 08 §7's own words (issue #41), with the O4 split #370 added.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SchemaDrift {
    pub type_name: String,
    /// Every producer observed serving the name, and what it claimed.
    pub servers: Vec<SchemaServer>,
    pub verdict: DriftVerdict,
}

/// A type the producer's slice references that its served describe set does
/// not cover — a violation of RFC 08 §7's totality clause.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TotalityGap {
    pub producer: String,
    pub missing: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The serialized `SchemaDrift` is a wire contract, and until #398 it had
    /// no pin at all — the one report shape in this file with none.
    ///
    /// The `origin` added there is the load-bearing half: a script reading a
    /// drift finding needs a host to act on, and a field that only *sometimes*
    /// appeared would be worse than one that never did.
    #[test]
    fn schema_drift_json_shape_is_pinned() {
        let drift = SchemaDrift {
            type_name: "Health".into(),
            servers: vec![
                SchemaServer {
                    producer: "sysinfo".into(),
                    origin: "h-3fa9c2d41b7e".into(),
                    hash: Asked::Asked("sha256:abc".into()),
                },
                SchemaServer {
                    producer: "sysinfo".into(),
                    origin: "h-8b1e07af22c9".into(),
                    // Served no identity: absent on the wire, never `null` and
                    // never `""` — the two spellings #370 pulled apart.
                    hash: Asked::NotAsked,
                },
            ],
            verdict: DriftVerdict::Disagree,
        };
        assert_eq!(
            serde_json::to_value(&drift).expect("serialize"),
            serde_json::json!({
                "type_name": "Health",
                "servers": [
                    {
                        "producer": "sysinfo",
                        "origin": "h-3fa9c2d41b7e",
                        "hash": "sha256:abc",
                    },
                    {
                        "producer": "sysinfo",
                        "origin": "h-8b1e07af22c9",
                    },
                ],
                "verdict": "disagree",
            })
        );
    }
}

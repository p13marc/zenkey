//! The interface plane: payload types and the carriers that move them —
//! one type's producers, subjects and media streams in one document.

use super::asked::Asked;
use super::schema::{SchemaDrift, SchemaRow};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct InterfaceTypeRow {
    pub name: String,
    pub carriers: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct InterfaceList {
    pub types: Vec<InterfaceTypeRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CarrierRow {
    pub producer: String,
    pub class: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InterfaceShow {
    pub type_name: String,
    pub carriers: Vec<CarrierRow>,
    /// What each producer serving this type name says its schema is
    /// (issue #51). `NotAsked` = `--schema` was not passed, so the bus was
    /// never asked; `Asked(vec![])` = asked and no carrier served one — the
    /// empty `Vec` used to conflate the two (RFC 09 §5.1 O4, review finding
    /// R4). Two rows with different hashes *is* the RFC 08 §7 drift finding,
    /// visible right here rather than only in `doctor`.
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub schemas: Asked<Vec<SchemaRow>>,
    /// The engine's verdict on whether the carriers agree about this type —
    /// [`schema_drift`](crate::model::decode::schema_drift) over the
    /// describe sweep, filtered to `type_name` (#410).
    ///
    /// Not derivable from `schemas`: a [`SchemaRow`] names a producer and
    /// no origin, and flattens an unserved hash to `""`, so two rows can
    /// neither show two hosts of one producer disagreeing nor tell "both
    /// said nothing" from "both said the same" (the O4 bug #370 fixed in the
    /// doctor). The renderer used to recompute drift over the rows and got
    /// exactly those two cases wrong; this field is the one implementation's
    /// answer, and the note reads it.
    ///
    /// Empty when nothing disagrees — and empty when `--schema` was not
    /// passed, which `schemas` already says (`NotAsked`); omitted from the
    /// document in both cases, so an unasked run does not sprout a field
    /// that reads as "asked, clean".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drift: Vec<SchemaDrift>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{DriftVerdict, SchemaServer};

    fn show(drift: Vec<SchemaDrift>) -> InterfaceShow {
        InterfaceShow {
            type_name: "Health".into(),
            carriers: vec![CarrierRow {
                producer: "sysinfo".into(),
                class: "state".into(),
                path: "health".into(),
            }],
            schemas: Asked::Asked(vec![SchemaRow {
                producer: "sysinfo".into(),
                type_name: "Health".into(),
                kind: "json-schema".into(),
                hash: "sha256:abc".into(),
                document: None,
            }]),
            drift,
        }
    }

    /// `drift` is additive to the pinned document (#410): absent when
    /// nothing disagrees, so a script keyed on the pre-#410 shape reads a
    /// clean run unchanged, and present as the engine's own `SchemaDrift`
    /// when something does.
    #[test]
    fn interface_show_json_shape_is_pinned_and_drift_is_absent_when_empty() {
        assert_eq!(
            serde_json::to_value(show(vec![])).expect("serialize"),
            serde_json::json!({
                "type_name": "Health",
                "carriers": [{"producer": "sysinfo", "class": "state", "path": "health"}],
                "schemas": [{
                    "producer": "sysinfo",
                    "type_name": "Health",
                    "kind": "json-schema",
                    "hash": "sha256:abc",
                }],
            })
        );
        let drifted = show(vec![SchemaDrift {
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
                    hash: Asked::Asked("sha256:def".into()),
                },
            ],
            verdict: DriftVerdict::Disagree,
        }]);
        let value = serde_json::to_value(drifted).expect("serialize");
        assert_eq!(
            value["drift"],
            serde_json::json!([{
                "type_name": "Health",
                "servers": [
                    {"producer": "sysinfo", "origin": "h-3fa9c2d41b7e", "hash": "sha256:abc"},
                    {"producer": "sysinfo", "origin": "h-8b1e07af22c9", "hash": "sha256:def"},
                ],
                "verdict": "disagree",
            }])
        );
    }
}

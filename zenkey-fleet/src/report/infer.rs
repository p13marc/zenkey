//! `registry infer` (#225, RFC 08 §6.1 v1.34): the observation-derived
//! registry draft as a document.
//!
//! The files `zenctl registry infer` writes derive from this report — the
//! emitter walks [`InferredProducer`]s — so `--format json` and the draft on
//! disk cannot disagree about what was inferred. Every field a
//! [`InferredSubject`] could not establish is **absent** (RFC 13 §3 O4
//! applied to a file, as §6.1 says), and every guess that is not a field
//! rides as a comment string, never laundered into the TOML as a value.

use serde::Serialize;

use super::asked::u64_is_zero;

/// What one inference run saw and what it drafted.
#[derive(Debug, Clone, Serialize)]
pub struct InferReport {
    /// The selector watched, or the `.zrec` path read.
    pub source: String,
    /// The window asked for, seconds. Absent for a capture — nothing was
    /// asked; the span is in the rows (O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_s: Option<f64>,
    /// First to last sample, seconds — the span every rate and refresh
    /// guess is measured over.
    pub span_s: f64,
    pub samples: u64,
    /// Samples the bounded observer missed (O6).
    pub dropped: u64,
    /// Distinct wire keys retained.
    pub keys_seen: usize,
    /// Keys refused at the key bound — the draft covers the retained set
    /// only (O6).
    pub keys_refused: u64,
    /// Distinct origins the retained keys came from. One origin cannot
    /// exhibit a leaf dimension; the draft says so when this is 1.
    pub origins: usize,
    /// Keys under the base that are not v1 keys — nothing to draft from.
    pub unparsed_keys: u64,
    /// Keys on a verbatim plane (`@rpc`, `@media`, `@blob`): no
    /// `[[subject]]` surface, so not drafted.
    pub off_plane_keys: u64,
    /// Samples carrying no structural document (fields unobservable).
    pub undocumented: u64,
    /// Samples past the observation limit, never read (distinct from
    /// `undocumented`, O6).
    #[serde(skip_serializing_if = "u64_is_zero")]
    pub unread: u64,
    /// Path observations refused at the path-table bound (O6).
    #[serde(skip_serializing_if = "u64_is_zero")]
    pub paths_refused: u64,
    /// Whether a registry slice set hinted the inference (var names,
    /// service ownership). The draft comes out without one, and says so.
    pub hinted_by_registry: bool,
    /// Run-level caveats the reviewer must read — stated as data so they
    /// reach json and ndjson.
    pub caveats: Vec<String>,
    pub producers: Vec<InferredProducer>,
}

/// One draft file: a producer (or service) and the subjects seen under it.
#[derive(Debug, Clone, Serialize)]
pub struct InferredProducer {
    /// The producer base name, or the service name (its origin without the
    /// `@`) — the draft file's stem.
    pub name: String,
    /// `Some("@catalog")` for a service origin (RFC 03 §1.5); the file then
    /// gets a `[service]` header.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_origin: Option<String>,
    pub subjects: Vec<InferredSubject>,
    pub types: Vec<InferredType>,
}

/// One inferred `[[subject]]`. Optional fields are absent when the
/// observation could not establish them, never defaulted.
#[derive(Debug, Clone, Serialize)]
pub struct InferredSubject {
    /// The inferred pattern, `{var}`s and a trailing `{rest...}` included.
    pub path: String,
    pub class: String,
    pub type_name: String,
    /// A generated-variant override, present only where two inferred paths
    /// would collide on the default name (`cpu/usage` beside
    /// `cpu/{v1}/usage`): the var-bearing one is then named with its vars,
    /// as the reference registry does by hand.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Present for the alert family only — `state` under a leading `alert`
    /// chunk — where RFC 08 §5 (v1.23) *requires* `qos = "alert"`. That is
    /// the family's normative profile, never what was observed: observed
    /// QoS stays a comment on every entry, this one included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qos: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_s: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// The guesses that are not fields — observed QoS, a `kind` guess, the
    /// sibling set a `{var}` was inferred from, what could not be
    /// established. Written above the entry as `#` lines.
    pub comments: Vec<String>,
    /// Distinct origins that published an expansion of this pattern.
    pub origins: usize,
    /// Distinct wire keys (expansions) the pattern covers.
    pub keys: usize,
    pub samples: u64,
}

/// One inferred payload type: a JSON Schema built from the observed
/// field kinds (`schema` absent when no sample was structural).
#[derive(Debug, Clone, Serialize)]
pub struct InferredType {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
    /// How many subjects bind this type (deduplicated by schema identity).
    pub subjects: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `zenctl registry infer --format json`'s document, pinned: every
    /// unestablished field is absent — a draft that defaulted one would be
    /// the lie RFC 08 §6.1 forbids, in JSON instead of TOML.
    #[test]
    fn infer_report_json_shape_is_pinned() {
        let report = InferReport {
            source: "v1/**".into(),
            window_s: Some(60.0),
            span_s: 59.2,
            samples: 120,
            dropped: 0,
            keys_seen: 2,
            keys_refused: 0,
            origins: 2,
            unparsed_keys: 0,
            off_plane_keys: 1,
            undocumented: 0,
            unread: 0,
            paths_refused: 0,
            hinted_by_registry: false,
            caveats: vec![],
            producers: vec![InferredProducer {
                name: "demo".into(),
                service_origin: None,
                subjects: vec![
                    InferredSubject {
                        path: "cpu/{v1}/usage_percent".into(),
                        class: "telemetry".into(),
                        type_name: "DemoCpuUsagePercent".into(),
                        variant: None,
                        unit: Some("percent".into()),
                        qos: None,
                        ttl_s: None,
                        rate: None,
                        cardinality: Some(10),
                        encoding: Some("application/json".into()),
                        comments: vec!["qos observed: sampled".into()],
                        origins: 2,
                        keys: 2,
                        samples: 118,
                    },
                    InferredSubject {
                        path: "health".into(),
                        class: "state".into(),
                        type_name: "DemoHealth".into(),
                        variant: None,
                        unit: None,
                        qos: None,
                        ttl_s: None,
                        rate: None,
                        cardinality: None,
                        encoding: None,
                        comments: vec![
                            "ttl_s not established: no refresh observed within 59.2 s".into(),
                        ],
                        origins: 2,
                        keys: 2,
                        samples: 2,
                    },
                ],
                types: vec![InferredType {
                    name: "DemoCpuUsagePercent".into(),
                    schema: Some(serde_json::json!({"type": "number"})),
                    subjects: 1,
                }],
            }],
        };
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::json!({
                "source": "v1/**",
                "window_s": 60.0,
                "span_s": 59.2,
                "samples": 120,
                "dropped": 0,
                "keys_seen": 2,
                "keys_refused": 0,
                "origins": 2,
                "unparsed_keys": 0,
                "off_plane_keys": 1,
                "undocumented": 0,
                "hinted_by_registry": false,
                "caveats": [],
                "producers": [{
                    "name": "demo",
                    "subjects": [
                        {
                            "path": "cpu/{v1}/usage_percent",
                            "class": "telemetry",
                            "type_name": "DemoCpuUsagePercent",
                            "unit": "percent",
                            "cardinality": 10,
                            "encoding": "application/json",
                            "comments": ["qos observed: sampled"],
                            "origins": 2,
                            "keys": 2,
                            "samples": 118,
                        },
                        {
                            "path": "health",
                            "class": "state",
                            "type_name": "DemoHealth",
                            "comments": [
                                "ttl_s not established: no refresh observed within 59.2 s",
                            ],
                            "origins": 2,
                            "keys": 2,
                            "samples": 2,
                        },
                    ],
                    "types": [{
                        "name": "DemoCpuUsagePercent",
                        "schema": {"type": "number"},
                        "subjects": 1,
                    }],
                }],
            }),
            "unestablished fields are absent, never null (RFC 08 §6.1, RFC 13 §3 O4)"
        );
    }
}

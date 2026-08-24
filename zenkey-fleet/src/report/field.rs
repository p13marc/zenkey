//! The field plane (#223): per-path observations inside a payload — what
//! moved, what never did, and which paths the cap turned away.

use super::asked::u64_is_zero;
use super::doctor::DoctorFinding;
use serde::Serialize;

/// One dotted path's statistics over a `zenctl field` window (#223) — the
/// per-field arrival story validation cannot tell.
#[derive(Debug, Clone, Serialize)]
pub struct FieldRow {
    /// The concrete wire key the path was observed under.
    pub key: String,
    /// The dotted path inside the structural value (`$` = a non-object root).
    pub path: String,
    /// Document samples in which the path was present.
    pub seen: u64,
    /// The key's document samples — the presence ratio's denominator.
    pub documents: u64,
    /// JSON kinds observed (one entry = type-stable).
    pub kinds: Vec<String>,
    /// Times the value differed from its previous observation.
    pub changes: u64,
    /// Window-relative seconds of the last change; absent = never changed
    /// within the window (which the stated window scopes — not "never").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_change_s: Option<f64>,
    /// Numeric min/max/last, present only when the path carried numbers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<f64>,
    /// Small-domain distinct values, total when present; absent = the domain
    /// outgrew the cap (stated, never silently partial).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
}

/// The `zenctl field` report (#223): bounded per-path statistics over one
/// window, plus the `field-vanished`/`field-stuck`/`field-new` findings.
/// Every bound states its cost (RFC 09 §5.1 O6), and "not asked" — no
/// registry, no served schema, no structural document — never reads as "no"
/// (O4).
#[derive(Debug, Clone, Serialize)]
pub struct FieldReport {
    pub selector: String,
    pub window_s: f64,
    pub samples: u64,
    pub keys_seen: usize,
    /// Samples the bounded observer missed (O6).
    pub dropped: u64,
    /// Samples carrying no structural document — fields unobservable for
    /// them, counted apart from absence (O4).
    pub undocumented: u64,
    /// Whether a registry was loaded: without one, declared `ttl_s` and type
    /// names are unknown and `field-stuck`/`field-new` are unjudgeable.
    pub registry_loaded: bool,
    /// Distinct (key, path) pairs tracked, against the bound they ran under.
    pub paths: usize,
    pub max_paths: usize,
    /// Path observations refused to stay within the bound (O6) — the table
    /// never truncates silently.
    pub paths_dropped: u64,
    /// Up to a handful of `key · path` names among the refused.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paths_dropped_examples: Vec<String>,
    /// Key projections the bounded facts cache (#107) retired during the
    /// window — non-zero means the declared-ttl/type context covers the
    /// retained keys only (RFC 09 §5.1 O6). Absent when zero.
    #[serde(skip_serializing_if = "u64_is_zero", default)]
    pub facts_evicted: u64,
    pub rows: Vec<FieldRow>,
    pub findings: Vec<DoctorFinding>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same contract again for `zenctl field --format json` (#223): the
    /// document changes only deliberately, and every "not asked" is an
    /// absent field, never a null or a zero (RFC 09 §5.1 O4).
    #[test]
    fn field_report_json_shape_is_pinned() {
        let report = FieldReport {
            selector: "v1/*/state/demo/health".into(),
            window_s: 30.0,
            samples: 40,
            keys_seen: 1,
            dropped: 0,
            undocumented: 2,
            registry_loaded: true,
            paths: 2,
            max_paths: 512,
            paths_dropped: 0,
            paths_dropped_examples: vec![],
            facts_evicted: 0,
            rows: vec![FieldRow {
                key: "v1/h-3fa9c2d41b7e/state/demo/health".into(),
                path: "temperature_c".into(),
                seen: 38,
                documents: 38,
                kinds: vec!["number".into()],
                changes: 0,
                last_change_s: None,
                min: Some(21.5),
                max: Some(21.5),
                last: Some(21.5),
                values: Some(vec!["21.5".into()]),
            }],
            findings: vec![],
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "selector": "v1/*/state/demo/health",
                "window_s": 30.0,
                "samples": 40,
                "keys_seen": 1,
                "dropped": 0,
                "undocumented": 2,
                "registry_loaded": true,
                "paths": 2,
                "max_paths": 512,
                "paths_dropped": 0,
                "rows": [{
                    "key": "v1/h-3fa9c2d41b7e/state/demo/health",
                    "path": "temperature_c",
                    "seen": 38,
                    "documents": 38,
                    "kinds": ["number"],
                    "changes": 0,
                    "min": 21.5,
                    "max": 21.5,
                    "last": 21.5,
                    "values": ["21.5"],
                }],
                "findings": [],
            }),
            "`last_change_s` and dropped-path examples are absent when there \
             is nothing to say, never null (O4); `values` absent would mean \
             the domain overflowed the cap"
        );
    }
}

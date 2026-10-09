//! The field plane (#223): per-path observations inside a payload — what
//! moved, what never did, and which paths the cap turned away. zk2's since
//! #612 (FJ8b): each payload decoded through its contract, and a path
//! judged against the paths its declared type declares.

use super::asked::u64_is_zero;
use super::doctor::DoctorSeverity;
use super::observe::LensScope;
use serde::Serialize;

/// One dotted path's statistics over a `zenctl field` window (#223) — the
/// per-field arrival story validation cannot tell.
#[derive(Debug, Clone, Serialize)]
pub struct FieldRow {
    /// The concrete wire key the path was observed under.
    pub key: String,
    /// The dotted path inside the decoded value (`$` = a non-object root).
    pub path: String,
    /// Whether the key's declared type accounts for the path: declared, or
    /// under a subtree it leaves open. Absent when no declared surface could
    /// be had — no contract resolved the key, or its type enumerates
    /// nothing — which is not "undeclared" (O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared: Option<bool>,
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

/// What a field finding says (#223), on the wire as its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum FieldCheck {
    /// A path seen, then absent while later payloads still decode.
    #[serde(rename = "field-vanished")]
    Vanished,
    /// A path the declared type never declares.
    #[serde(rename = "field-new")]
    New,
}

impl FieldCheck {
    /// The wire id, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            FieldCheck::Vanished => "field-vanished",
            FieldCheck::New => "field-new",
        }
    }
}

/// One field finding: an observation with a stated window, never a verdict
/// on the producer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldFinding {
    pub severity: DoctorSeverity,
    pub check: FieldCheck,
    /// `<key> · <path>`, or `fleet` for a capped tail.
    pub subject: String,
    pub evidence: String,
}

/// The `zenctl field` report (#223): bounded per-path statistics over one
/// window, plus the `field-vanished`/`field-new` findings. Every bound
/// states its cost (O6), and "not asked" — no contract, no declared surface,
/// no document — never reads as "no" (O4). `field-stuck` is not asked: it
/// judges against a declared freshness, the `freshness.v1` profile's (#613).
#[derive(Debug, Clone, Serialize)]
pub struct FieldReport {
    /// The wire selector watched, on a session in no namespace.
    pub selector: String,
    pub window_s: f64,
    /// What the keys were resolved and decoded with.
    pub lens: LensScope,
    pub samples: u64,
    pub keys_seen: usize,
    /// Samples the bounded observer missed (O6).
    pub dropped: u64,
    /// Samples carrying no document — a raw type, bytes that do not decode
    /// as their type, a deletion — fields unobservable for them, counted
    /// apart from absence (O4).
    pub undocumented: u64,
    /// Samples whose payload was past the observation limit and therefore
    /// never read — distinct from `undocumented`, which means the payload was
    /// read and carried no document (O6).
    #[serde(skip_serializing_if = "u64_is_zero")]
    pub unread: u64,
    /// Samples whose key no contract resolved: observed structurally, and
    /// judged for nothing that needs a declared type (O4).
    pub unresolved: u64,
    /// Distinct (key, path) pairs tracked, against the bound they ran under.
    pub paths: usize,
    pub max_paths: usize,
    /// Path observations refused to stay within the bound (O6) — the table
    /// never truncates silently.
    pub paths_dropped: u64,
    /// Up to a handful of `key · path` names among the refused.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paths_dropped_examples: Vec<String>,
    pub rows: Vec<FieldRow>,
    pub findings: Vec<FieldFinding>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same contract again for `zenctl field --format json` (#223): the
    /// document changes only deliberately, and every "not asked" is an
    /// absent field, never a null or a zero (O4).
    #[test]
    fn field_report_json_shape_is_pinned() {
        let report = FieldReport {
            selector: "zk2/*/tc/tc.netif.v1/state/**".into(),
            window_s: 30.0,
            lens: LensScope {
                namespace: String::new(),
                presence: None,
                contracts: 1,
            },
            samples: 40,
            keys_seen: 1,
            dropped: 0,
            undocumented: 2,
            unread: 0,
            unresolved: 0,
            paths: 2,
            max_paths: 512,
            paths_dropped: 0,
            paths_dropped_examples: vec![],
            rows: vec![FieldRow {
                key: "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0".into(),
                path: "mtu".into(),
                declared: Some(true),
                seen: 38,
                documents: 38,
                kinds: vec!["number".into()],
                changes: 0,
                last_change_s: None,
                min: Some(1500.0),
                max: Some(1500.0),
                last: Some(1500.0),
                values: Some(vec!["1500".into()]),
            }],
            findings: vec![FieldFinding {
                severity: DoctorSeverity::Warning,
                check: FieldCheck::New,
                subject: "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0 · extra".into(),
                evidence: "never declared".into(),
            }],
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "selector": "zk2/*/tc/tc.netif.v1/state/**",
                "window_s": 30.0,
                "lens": {"namespace": "", "contracts": 1},
                "samples": 40,
                "keys_seen": 1,
                "dropped": 0,
                "undocumented": 2,
                "unresolved": 0,
                "paths": 2,
                "max_paths": 512,
                "paths_dropped": 0,
                "rows": [{
                    "key": "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0",
                    "path": "mtu",
                    "declared": true,
                    "seen": 38,
                    "documents": 38,
                    "kinds": ["number"],
                    "changes": 0,
                    "min": 1500.0,
                    "max": 1500.0,
                    "last": 1500.0,
                    "values": ["1500"],
                }],
                "findings": [{
                    "severity": "warning",
                    "check": "field-new",
                    "subject": "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0 · extra",
                    "evidence": "never declared",
                }],
            }),
            "`last_change_s` and dropped-path examples are absent when there \
             is nothing to say, never null (O4); `declared` absent would mean \
             no declared surface; `values` absent, the domain overflowed"
        );
    }
}

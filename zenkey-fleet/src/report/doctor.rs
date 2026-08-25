//! The doctor plane: conformance findings, and the observation that
//! produced them.
//!
//! [`ObservationSummary`] is not decoration. A finding is only as good as
//! the window it was found in, so the document carries what was watched, for
//! how long, and — crucially — what was **dropped** (RFC 09 §5.1 O6): a
//! clean report over a lossy window is not a clean fleet.

use std::fmt;

use super::asked::{Asked, u64_is_zero};
use serde::{Deserialize, Serialize};

/// Every check [`run_doctor`](crate::judge::doctor::run_doctor) can emit.
///
/// **Stable API**: scripts key on these through `--format json`, and the GUI
/// keys deltas on them. New checks append; nothing renames one — which is
/// exactly why this is an enum and no longer a `[&str; 21]` beside a
/// `check: String`. The wire spelling is unchanged (kebab-case, one token per
/// variant), and `check_ids_are_stable` still pins the list (#347).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckId {
    SliceParse,
    SliceSync,
    IntrospectCoverage,
    AdminUnreachable,
    RouterVersionSkew,
    DescribeTotality,
    SchemaDrift,
    DescribeMissing,
    StaleState,
    UnstampedState,
    StorageCoverage,
    // The `--for` passive phase (#161) — traffic judged as it rides.
    PayloadUndecodable,
    PayloadInvalid,
    QosObservedMismatch,
    UnregisteredTraffic,
    RateOverDeclared,
    TimestampStampedElsewhere,
    /// Key-population budgets (#221): declared `cardinality` vs the observed
    /// expansion count, per origin. `{path...}` families are exempt and say so.
    CardinalityOverDeclared,
    // Field intelligence (#223): per-dotted-path judgement over the listen
    // window — the failure modes per-sample validation cannot see.
    FieldVanished,
    FieldStuck,
    FieldNew,
}

impl CheckId {
    /// Every check id, in the order the doctor reports them.
    pub const ALL: [CheckId; 21] = [
        CheckId::SliceParse,
        CheckId::SliceSync,
        CheckId::IntrospectCoverage,
        CheckId::AdminUnreachable,
        CheckId::RouterVersionSkew,
        CheckId::DescribeTotality,
        CheckId::SchemaDrift,
        CheckId::DescribeMissing,
        CheckId::StaleState,
        CheckId::UnstampedState,
        CheckId::StorageCoverage,
        CheckId::PayloadUndecodable,
        CheckId::PayloadInvalid,
        CheckId::QosObservedMismatch,
        CheckId::UnregisteredTraffic,
        CheckId::RateOverDeclared,
        CheckId::TimestampStampedElsewhere,
        CheckId::CardinalityOverDeclared,
        CheckId::FieldVanished,
        CheckId::FieldStuck,
        CheckId::FieldNew,
    ];

    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            CheckId::SliceParse => "slice-parse",
            CheckId::SliceSync => "slice-sync",
            CheckId::IntrospectCoverage => "introspect-coverage",
            CheckId::AdminUnreachable => "admin-unreachable",
            CheckId::RouterVersionSkew => "router-version-skew",
            CheckId::DescribeTotality => "describe-totality",
            CheckId::SchemaDrift => "schema-drift",
            CheckId::DescribeMissing => "describe-missing",
            CheckId::StaleState => "stale-state",
            CheckId::UnstampedState => "unstamped-state",
            CheckId::StorageCoverage => "storage-coverage",
            CheckId::PayloadUndecodable => "payload-undecodable",
            CheckId::PayloadInvalid => "payload-invalid",
            CheckId::QosObservedMismatch => "qos-observed-mismatch",
            CheckId::UnregisteredTraffic => "unregistered-traffic",
            CheckId::RateOverDeclared => "rate-over-declared",
            CheckId::TimestampStampedElsewhere => "timestamp-stamped-elsewhere",
            CheckId::CardinalityOverDeclared => "cardinality-over-declared",
            CheckId::FieldVanished => "field-vanished",
            CheckId::FieldStuck => "field-stuck",
            CheckId::FieldNew => "field-new",
        }
    }

    /// Read a check id a caller supplied — `doctor --transitions`, a
    /// `check expect` condition, a script's filter.
    pub fn parse(token: &str) -> Option<CheckId> {
        CheckId::ALL.into_iter().find(|c| c.as_str() == token)
    }
}

impl fmt::Display for CheckId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How bad a doctor finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorSeverity {
    /// A contract violation — the fleet disagrees with the RFCs or with
    /// itself.
    Error,
    /// Suspicious but explainable — judgement is degraded, not wrong.
    Warning,
    /// Worth knowing; not a defect.
    Info,
}

/// One machine-readable doctor finding (issue #46): what check fired, on
/// what, with the evidence and the normative citation — the shape the GUI
/// doctor panel renders as-is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorFinding {
    pub severity: DoctorSeverity,
    /// Which check fired. Serializes to the same kebab-case token it always
    /// has; it is a type now so a typo is a compile error rather than a
    /// finding nothing matches (#347).
    pub check: CheckId,
    /// What the finding is about (producer, key, or mesh-level subject).
    pub subject: String,
    /// The observed evidence, human-readable.
    pub evidence: String,
    /// The RFC section that makes this a finding (`None` when the check is
    /// operational judgement rather than a normative clause).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation: Option<String>,
}

/// The full doctor run: findings plus the coverage summary that makes an
/// empty findings list legible (what was checked, not just what was found —
/// RFC 05 §3.1: silence needs attribution).
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub findings: Vec<DoctorFinding>,
    /// Producer slices confirmed in sync with the local registry
    /// (`origin/producer`).
    ///
    /// `NotAsked` = no local registry was given, so the served-vs-declared
    /// diff **never ran** — which must not read like "ran, none in sync"
    /// (RFC 09 §5.1 O4). `Asked(vec![])` = the diff ran and confirmed
    /// nothing; the findings say why. The `Vec` used to skip-if-empty, which
    /// conflated the two (review finding R1); the `Option` that fixed it is
    /// now [`Asked`], wire-identically (#246 / P1).
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub synced: Asked<Vec<String>>,
    /// Introspect replies received across the fleet.
    pub introspect_answered: usize,
    /// Producers on the liveliness roster.
    pub live_producers: usize,
    /// Producers serving an RFC 08 §7 `describe`.
    pub describe_served: usize,
    /// Producers serving no `describe` (a SHOULD, not a MUST).
    pub describe_missing: usize,
    /// Routers that answered the admin sweep.
    pub routers: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub router_version: Option<String>,
    /// Whether the `--deep` freshness/storage checks ran.
    pub deep: bool,
    /// The passive listening phase (`--for`, #161) — absent when it did
    /// not run, so pre-#161 JSON consumers see an unchanged document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation: Option<ObservationSummary>,
}

/// What `doctor --for` observed (#161) — the scope statement that keeps
/// its findings honest (O5: `**` never crosses an `@`-chunk, so this section
/// names exactly which selectors were watched), and the drop count that
/// taints them (O6).
#[derive(Debug, Clone, Serialize)]
pub struct ObservationSummary {
    pub window_s: f64,
    /// The selectors actually watched — coverage is a statement, not a vibe.
    pub scopes: Vec<String>,
    pub samples: u64,
    pub keys_seen: usize,
    /// Samples the bounded observer missed; non-zero weakens every
    /// listen-phase finding and the report says so.
    pub dropped: u64,
    /// Samples carrying the synthetic-traffic marker (RFC 09 §5.3, #162) —
    /// generated traffic judged as real would be a self-inflicted finding.
    pub synthetic_marked: u64,
    /// Field-intelligence paths (#223) the bounded per-path table refused to
    /// track — the O6 cost of that bound, absent when zero so pre-#223
    /// consumers see an unchanged document.
    #[serde(skip_serializing_if = "u64_is_zero")]
    pub field_paths_dropped: u64,
    /// Key projections the bounded facts cache (#107) retired during the
    /// window — non-zero means `keys_seen`, the budget sweep and the field
    /// context cover the retained keys only, and the report says what the
    /// bound cost (RFC 09 §5.1 O6). Absent when zero, so earlier JSON
    /// consumers see an unchanged document.
    #[serde(skip_serializing_if = "u64_is_zero", default)]
    pub facts_evicted: u64,
}

impl DoctorReport {
    pub fn count(&self, severity: DoctorSeverity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Asked;

    /// The serialized DoctorReport is a wire contract: `zenctl doctor
    /// --format json` scripts and the GUI panel both consume this exact
    /// shape. Field renames/removals break users — this golden pin makes
    /// that a deliberate act.
    #[test]
    fn doctor_report_json_shape_is_pinned() {
        let report = DoctorReport {
            findings: vec![DoctorFinding {
                severity: DoctorSeverity::Error,
                check: CheckId::SliceSync,
                subject: "h-3fa9c2d41b7e/sysinfo".into(),
                evidence: "registry version differs: served 1.0, local 2.0".into(),
                citation: Some("RFC 08 §6".into()),
            }],
            // R1: `Option` since the report-honesty batch — `Some` serializes
            // exactly as the old non-empty `Vec` did.
            synced: Asked::Asked(vec!["h-3fa9c2d41b7e/other (registry 1.0)".into()]),
            introspect_answered: 2,
            live_producers: 3,
            describe_served: 1,
            describe_missing: 1,
            routers: 1,
            router_version: Some("1.9.0".into()),
            deep: false,
            observation: None,
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "findings": [{
                    "severity": "error",
                    "check": "slice-sync",
                    "subject": "h-3fa9c2d41b7e/sysinfo",
                    "evidence": "registry version differs: served 1.0, local 2.0",
                    "citation": "RFC 08 §6",
                }],
                "synced": ["h-3fa9c2d41b7e/other (registry 1.0)"],
                "introspect_answered": 2,
                "live_producers": 3,
                "describe_served": 1,
                "describe_missing": 1,
                "routers": 1,
                "router_version": "1.9.0",
                "deep": false,
            }),
            "without --for the document is byte-identical to pre-#161"
        );
        // R1 (report-honesty batch): `synced` is three-state. Absent = the
        // served-vs-declared diff never ran (no registry, O4); `[]` = it ran
        // and confirmed nothing; non-empty pins above. The wire change is
        // deliberate: a no-registry run serialized nothing here before, and
        // still does — only the ran-and-empty case gains a visible `[]`.
        let unchecked = DoctorReport {
            synced: Asked::NotAsked,
            ..report.clone()
        };
        let json = serde_json::to_value(&unchecked).unwrap();
        assert!(
            !json.as_object().unwrap().contains_key("synced"),
            "diff never ran: the key is absent, exactly as pre-R1 no-registry \
             runs serialized"
        );
        let ran_empty = DoctorReport {
            synced: Asked::Asked(vec![]),
            ..report.clone()
        };
        let json = serde_json::to_value(&ran_empty).unwrap();
        assert_eq!(
            json["synced"],
            serde_json::json!([]),
            "ran and confirmed nothing is `[]`, not absence"
        );
        // With the listen phase, the observation section pins too. Note
        // `field_paths_dropped` (#223) is absent at zero — appended, like
        // #213/#221's additions, so pre-#223 consumers see an unchanged
        // document.
        let report = DoctorReport {
            observation: Some(ObservationSummary {
                window_s: 10.0,
                scopes: vec!["v1/*/state/**".into()],
                samples: 42,
                keys_seen: 7,
                dropped: 0,
                synthetic_marked: 3,
                field_paths_dropped: 0,
                facts_evicted: 0,
            }),
            ..report
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json["observation"],
            serde_json::json!({
                "window_s": 10.0,
                "scopes": ["v1/*/state/**"],
                "samples": 42,
                "keys_seen": 7,
                "dropped": 0,
                "synthetic_marked": 3,
            })
        );
        // …and pins by name when the field table did drop (O6 is a wire
        // fact, not only a table note).
        let report = DoctorReport {
            observation: Some(ObservationSummary {
                field_paths_dropped: 2,
                ..report.observation.unwrap()
            }),
            ..report
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["observation"]["field_paths_dropped"], 2);
        // The facts-cache eviction count (#107) follows the same append
        // rule: absent at zero, pinned by name when the bound cost keys.
        assert!(
            !json["observation"]
                .as_object()
                .unwrap()
                .contains_key("facts_evicted")
        );
        let report = DoctorReport {
            observation: Some(ObservationSummary {
                facts_evicted: 5,
                ..report.observation.unwrap()
            }),
            ..report
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["observation"]["facts_evicted"], 5);
    }
}

#[cfg(test)]
mod check_id_tests {
    use super::*;

    /// The id vocabulary is API: additions append, nothing renames. If this
    /// test fails you are renaming a shipped check id — don't.
    ///
    /// It asserts the *wire* spelling, not the variant names, which is the
    /// half that is promised: #347 turned a `[&str; 21]` into an enum, and
    /// this is what proves the turn cost nothing on the wire.
    #[test]
    fn check_ids_are_stable() {
        assert_eq!(
            CheckId::ALL.map(CheckId::as_str),
            [
                "slice-parse",
                "slice-sync",
                "introspect-coverage",
                "admin-unreachable",
                "router-version-skew",
                "describe-totality",
                "schema-drift",
                "describe-missing",
                "stale-state",
                "unstamped-state",
                "storage-coverage",
                "payload-undecodable",
                "payload-invalid",
                "qos-observed-mismatch",
                "unregistered-traffic",
                "rate-over-declared",
                "timestamp-stamped-elsewhere",
                "cardinality-over-declared",
                "field-vanished",
                "field-stuck",
                "field-new",
            ]
        );
    }

    /// `as_str`, serde and `parse` are one vocabulary, not three.
    #[test]
    fn every_check_id_round_trips_through_serde_and_parse() {
        for id in CheckId::ALL {
            let json = serde_json::to_string(&id).unwrap();
            assert_eq!(json, format!("\"{}\"", id.as_str()));
            assert_eq!(serde_json::from_str::<CheckId>(&json).unwrap(), id);
            assert_eq!(CheckId::parse(id.as_str()), Some(id));
        }
        assert_eq!(CheckId::parse("slice-sinc"), None);
    }
}

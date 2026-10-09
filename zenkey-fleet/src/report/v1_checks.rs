//! The doctor plane: conformance findings, and the observation that
//! produced them.
//!
//! [`ObservationSummary`] is not decoration. A finding is only as good as
//! the window it was found in, so the document carries what was watched, for
//! how long, and — crucially — what was **dropped** (RFC 09 §5.1 O6): a
//! clean report over a lossy window is not a clean fleet.

use std::fmt;

use super::asked::{Asked, u64_is_zero};
use super::judgement::Judgement;
use serde::{Deserialize, Serialize};

/// Every check [`run_v1_doctor`](crate::judge::registry_checks::run_v1_doctor) can emit.
///
/// **Stable API**: scripts key on these through `--format json`, and the GUI
/// keys deltas on them. New checks append; nothing renames one — which is
/// exactly why this is an enum and no longer a `[&str; 21]` beside a
/// `check: String`. The wire spelling is unchanged (kebab-case, one token per
/// variant), and `check_ids_are_stable` still pins the list (#347).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum V1CheckId {
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
    /// Declared versus observed (#422, RFC 08 §2 v1.32, RFC 13 §3): a
    /// self-describing payload's tag disagrees with the subject's declared
    /// `kind`, or a `counter` decreased within one origin's series with no
    /// `alive` cycle in between.
    KindMismatch,
    /// A producer's declared `[budget]` (#391, RFC 08 §2 v1.32, RFC 13 §3)
    /// against the `self_stats` on its health document (RFC 04 §1.2): the
    /// resident set over `rss_mb`, or a named table over its bound. Asked
    /// under `--deep`, because a health fetch costs the data plane.
    BudgetExceeded,
}

impl V1CheckId {
    /// Every check id, in the order the doctor reports them.
    pub const ALL: [V1CheckId; 23] = [
        V1CheckId::SliceParse,
        V1CheckId::SliceSync,
        V1CheckId::IntrospectCoverage,
        V1CheckId::AdminUnreachable,
        V1CheckId::RouterVersionSkew,
        V1CheckId::DescribeTotality,
        V1CheckId::SchemaDrift,
        V1CheckId::DescribeMissing,
        V1CheckId::StaleState,
        V1CheckId::UnstampedState,
        V1CheckId::StorageCoverage,
        V1CheckId::PayloadUndecodable,
        V1CheckId::PayloadInvalid,
        V1CheckId::QosObservedMismatch,
        V1CheckId::UnregisteredTraffic,
        V1CheckId::RateOverDeclared,
        V1CheckId::TimestampStampedElsewhere,
        V1CheckId::CardinalityOverDeclared,
        V1CheckId::FieldVanished,
        V1CheckId::FieldStuck,
        V1CheckId::FieldNew,
        V1CheckId::KindMismatch,
        V1CheckId::BudgetExceeded,
    ];

    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            V1CheckId::SliceParse => "slice-parse",
            V1CheckId::SliceSync => "slice-sync",
            V1CheckId::IntrospectCoverage => "introspect-coverage",
            V1CheckId::AdminUnreachable => "admin-unreachable",
            V1CheckId::RouterVersionSkew => "router-version-skew",
            V1CheckId::DescribeTotality => "describe-totality",
            V1CheckId::SchemaDrift => "schema-drift",
            V1CheckId::DescribeMissing => "describe-missing",
            V1CheckId::StaleState => "stale-state",
            V1CheckId::UnstampedState => "unstamped-state",
            V1CheckId::StorageCoverage => "storage-coverage",
            V1CheckId::PayloadUndecodable => "payload-undecodable",
            V1CheckId::PayloadInvalid => "payload-invalid",
            V1CheckId::QosObservedMismatch => "qos-observed-mismatch",
            V1CheckId::UnregisteredTraffic => "unregistered-traffic",
            V1CheckId::RateOverDeclared => "rate-over-declared",
            V1CheckId::TimestampStampedElsewhere => "timestamp-stamped-elsewhere",
            V1CheckId::CardinalityOverDeclared => "cardinality-over-declared",
            V1CheckId::FieldVanished => "field-vanished",
            V1CheckId::FieldStuck => "field-stuck",
            V1CheckId::FieldNew => "field-new",
            V1CheckId::KindMismatch => "kind-mismatch",
            V1CheckId::BudgetExceeded => "budget-exceeded",
        }
    }

    /// Read a check id a caller supplied — `doctor --transitions`, a
    /// `check expect` condition, a script's filter.
    pub fn parse(token: &str) -> Option<V1CheckId> {
        V1CheckId::ALL.into_iter().find(|c| c.as_str() == token)
    }
}

impl fmt::Display for V1CheckId {
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

impl DoctorSeverity {
    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            DoctorSeverity::Error => "error",
            DoctorSeverity::Warning => "warning",
            DoctorSeverity::Info => "info",
        }
    }

    /// Whether this severity reaches `floor` — `Error` reaches every floor,
    /// `Info` only its own. The ladder a severity threshold reads (#510).
    pub fn reaches(self, floor: DoctorSeverity) -> bool {
        fn rank(s: DoctorSeverity) -> u8 {
            match s {
                DoctorSeverity::Error => 2,
                DoctorSeverity::Warning => 1,
                DoctorSeverity::Info => 0,
            }
        }
        rank(self) >= rank(floor)
    }
}

/// One machine-readable doctor finding (issue #46): what check fired, on
/// what, with the evidence and the normative citation — the shape the GUI
/// doctor panel renders as-is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V1Finding {
    pub severity: DoctorSeverity,
    /// Which check fired. Serializes to the same kebab-case token it always
    /// has; it is a type now so a typo is a compile error rather than a
    /// finding nothing matches (#347).
    pub check: V1CheckId,
    /// What the finding is about (producer, key, or mesh-level subject).
    pub subject: String,
    /// The observed evidence, human-readable.
    pub evidence: String,
    /// The RFC section that makes this a finding (`None` when the check is
    /// operational judgement rather than a normative clause).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation: Option<String>,
}

/// What one doctor run says relative to the previous one (#389): findings
/// keyed on `(check, subject)`, so evidence and severity drift count as
/// unchanged. Computed by [`crate::v1_doctor_delta`]; rendered by the GUI
/// panel and routed by a notifier's `doctor` rule.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct V1DoctorDelta {
    /// Findings present now and absent from the previous run.
    pub new: Vec<V1Finding>,
    /// Findings present in the previous run and gone now.
    pub fixed: Vec<V1Finding>,
    /// Findings present in both runs.
    pub unchanged: usize,
}

impl V1DoctorDelta {
    /// Whether `f` is one of the new findings, by its key.
    pub fn is_new(&self, f: &V1Finding) -> bool {
        self.new
            .iter()
            .any(|n| n.check == f.check && n.subject == f.subject)
    }
}

/// The full doctor run: findings plus the coverage summary that makes an
/// empty findings list legible (what was checked, not just what was found —
/// RFC 05 §3.1: silence needs attribution).
#[derive(Debug, Clone, Serialize)]
pub struct V1DoctorReport {
    pub findings: Vec<V1Finding>,
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
    /// Introspect replies received across the fleet — readable or not: an
    /// unreadable one was received, and is a `slice-parse` finding, never
    /// counted as silence (#491, RFC 13 §3 O4).
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
    /// Set when the run **judged nothing** (#510), with the reason: no
    /// producer on the liveliness roster, no router answering the admin
    /// space, and nothing else that answered or rode — every check ran over
    /// an empty scope, so its silence is not a clean bill (RFC 13 §1.2's
    /// `Unobservable`; RFC 05 §3.1, silence needs attribution). A doctor
    /// pointed at the wrong endpoint or base used to look exactly like a
    /// healthy fleet. Absent whenever something was in scope, so an ordinary
    /// document is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unobservable: Option<String>,
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

impl V1DoctorReport {
    pub fn count(&self, severity: DoctorSeverity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }

    /// The run as an RFC 13 §1.2 [`Judgement`], under an optional severity
    /// threshold — what a frontend exits through (#510).
    ///
    /// The judged claim is *"the fleet has a finding at or above
    /// `threshold`"*, so a hit is `Established` and none is
    /// `NotEstablished`. With no threshold, findings are output rather than
    /// verdicts and the run is `NotEstablished` whatever it found. A run that
    /// [judged nothing](V1DoctorReport::unobservable) is `Unobservable` under
    /// every threshold, `None` included: no threshold turns an empty scope
    /// into a healthy fleet.
    pub fn judgement(&self, threshold: Option<DoctorSeverity>) -> Judgement {
        if let Some(reason) = &self.unobservable {
            return Judgement::Unobservable {
                reason: reason.clone(),
            };
        }
        let Some(floor) = threshold else {
            return Judgement::NotEstablished {
                reason: "no severity threshold: findings are output, not verdicts".into(),
            };
        };
        if self.findings.iter().any(|f| f.severity.reaches(floor)) {
            Judgement::Established
        } else {
            Judgement::NotEstablished {
                reason: format!("no finding at or above {}", floor.as_str()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Asked;

    /// The serialized V1DoctorReport is a wire contract: `zenctl doctor
    /// --format json` scripts and the GUI panel both consume this exact
    /// shape. Field renames/removals break users — this golden pin makes
    /// that a deliberate act.
    #[test]
    fn doctor_report_json_shape_is_pinned() {
        let report = V1DoctorReport {
            findings: vec![V1Finding {
                severity: DoctorSeverity::Error,
                check: V1CheckId::SliceSync,
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
            unobservable: None,
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
        let unchecked = V1DoctorReport {
            synced: Asked::NotAsked,
            ..report.clone()
        };
        let json = serde_json::to_value(&unchecked).unwrap();
        assert!(
            !json.as_object().unwrap().contains_key("synced"),
            "diff never ran: the key is absent, exactly as pre-R1 no-registry \
             runs serialized"
        );
        let ran_empty = V1DoctorReport {
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
        let report = V1DoctorReport {
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
        let report = V1DoctorReport {
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
        let report = V1DoctorReport {
            observation: Some(ObservationSummary {
                facts_evicted: 5,
                ..report.observation.unwrap()
            }),
            ..report
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["observation"]["facts_evicted"], 5);
        // #510: a run that judged nothing says so, by name and with its
        // reason — appended, absent otherwise, like every addition above.
        assert!(!json.as_object().unwrap().contains_key("unobservable"));
        let report = V1DoctorReport {
            unobservable: Some("nothing in scope".into()),
            ..report
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["unobservable"], "nothing in scope");
    }

    /// #510: the run's judgement under a threshold — a hit is the finding
    /// (1), none is clean (0), no threshold is always clean, and a run that
    /// judged nothing is unobservable (2) under every threshold, `None`
    /// included.
    #[test]
    fn the_judgement_reads_the_threshold_and_the_empty_scope() {
        use crate::report::judgement_exit_code;
        let warning = V1Finding {
            severity: DoctorSeverity::Warning,
            check: V1CheckId::SchemaDrift,
            subject: "Health".into(),
            evidence: "agreement cannot be judged".into(),
            citation: None,
        };
        let report = V1DoctorReport {
            findings: vec![warning],
            synced: Asked::NotAsked,
            introspect_answered: 1,
            live_producers: 1,
            describe_served: 1,
            describe_missing: 0,
            routers: 0,
            router_version: None,
            deep: false,
            observation: None,
            unobservable: None,
        };
        let exit = |r: &V1DoctorReport, t| judgement_exit_code(&r.judgement(t));
        assert_eq!(exit(&report, None), 0);
        assert_eq!(exit(&report, Some(DoctorSeverity::Error)), 0);
        assert_eq!(exit(&report, Some(DoctorSeverity::Warning)), 1);
        assert_eq!(exit(&report, Some(DoctorSeverity::Info)), 1);

        let empty = V1DoctorReport {
            findings: vec![],
            live_producers: 0,
            introspect_answered: 0,
            describe_served: 0,
            unobservable: Some("nothing in scope".into()),
            ..report
        };
        for t in [
            None,
            Some(DoctorSeverity::Error),
            Some(DoctorSeverity::Warning),
            Some(DoctorSeverity::Info),
        ] {
            assert_eq!(exit(&empty, t), 2, "{t:?}");
        }
        assert_eq!(
            empty.judgement(None),
            Judgement::Unobservable {
                reason: "nothing in scope".into()
            }
        );
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
            V1CheckId::ALL.map(V1CheckId::as_str),
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
                "kind-mismatch",
                "budget-exceeded",
            ]
        );
    }

    /// `as_str`, serde and `parse` are one vocabulary, not three.
    #[test]
    fn every_check_id_round_trips_through_serde_and_parse() {
        for id in V1CheckId::ALL {
            let json = serde_json::to_string(&id).unwrap();
            assert_eq!(json, format!("\"{}\"", id.as_str()));
            assert_eq!(serde_json::from_str::<V1CheckId>(&json).unwrap(), id);
            assert_eq!(V1CheckId::parse(id.as_str()), Some(id));
        }
        assert_eq!(V1CheckId::parse("slice-sinc"), None);
    }
}

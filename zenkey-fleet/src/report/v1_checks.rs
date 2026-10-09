//! v1's check vocabulary (#612, FJ6): the ids and findings of the registry
//! checks `check conform` projects (`judge::registry_checks`) and of
//! `field`'s per-path judgements — renamed out of the `doctor` domain when
//! zk2's doctor took it, and cut to what those two still produce.
//!
//! [`ObservationSummary`] is not decoration. A finding is only as good as
//! the window it was found in, so the document carries what was watched, for
//! how long, and — crucially — what was **dropped** (RFC 09 §5.1 O6): a
//! clean report over a lossy window is not a clean fleet. `check conform`
//! carries it on its report.

use std::fmt;

use super::asked::u64_is_zero;
use super::doctor::DoctorSeverity;
use serde::{Deserialize, Serialize};

/// Every check v1's registry checks and `field` can emit.
///
/// **Stable API** for the verbs that still read it: scripts key on these
/// through `field --format json`, and `check conform` maps them onto its
/// assertions. The wire spelling is kebab-case, one token per variant, and
/// `check_ids_are_stable` pins the list (#347). The ids the doctor verb
/// alone filed went with it (FJ6); the `v1` branch keeps them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum V1CheckId {
    SliceParse,
    SliceSync,
    DescribeTotality,
    SchemaDrift,
    StaleState,
    // The passive listen phase (#161) — traffic judged as it rides.
    PayloadUndecodable,
    PayloadInvalid,
    QosObservedMismatch,
    UnregisteredTraffic,
    RateOverDeclared,
    /// Key-population budgets (#221): declared `cardinality` vs the observed
    /// expansion count, per origin. `{path...}` families are exempt and say so.
    CardinalityOverDeclared,
    /// Declared versus observed (#422, RFC 08 §2 v1.32, RFC 13 §3): a
    /// self-describing payload's tag disagrees with the subject's declared
    /// `kind`, or a `counter` decreased within one origin's series with no
    /// `alive` cycle in between.
    KindMismatch,
    /// A producer's declared `[budget]` (#391, RFC 08 §2 v1.32, RFC 13 §3)
    /// against the `self_stats` on its health document (RFC 04 §1.2): the
    /// resident set over `rss_mb`, or a named table over its bound. Asked
    /// under `deep`, because a health fetch costs the data plane.
    BudgetExceeded,
}

impl V1CheckId {
    /// Every check id.
    pub const ALL: [V1CheckId; 13] = [
        V1CheckId::SliceParse,
        V1CheckId::SliceSync,
        V1CheckId::DescribeTotality,
        V1CheckId::SchemaDrift,
        V1CheckId::StaleState,
        V1CheckId::PayloadUndecodable,
        V1CheckId::PayloadInvalid,
        V1CheckId::QosObservedMismatch,
        V1CheckId::UnregisteredTraffic,
        V1CheckId::RateOverDeclared,
        V1CheckId::CardinalityOverDeclared,
        V1CheckId::KindMismatch,
        V1CheckId::BudgetExceeded,
    ];

    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            V1CheckId::SliceParse => "slice-parse",
            V1CheckId::SliceSync => "slice-sync",
            V1CheckId::DescribeTotality => "describe-totality",
            V1CheckId::SchemaDrift => "schema-drift",
            V1CheckId::StaleState => "stale-state",
            V1CheckId::PayloadUndecodable => "payload-undecodable",
            V1CheckId::PayloadInvalid => "payload-invalid",
            V1CheckId::QosObservedMismatch => "qos-observed-mismatch",
            V1CheckId::UnregisteredTraffic => "unregistered-traffic",
            V1CheckId::RateOverDeclared => "rate-over-declared",
            V1CheckId::CardinalityOverDeclared => "cardinality-over-declared",
            V1CheckId::KindMismatch => "kind-mismatch",
            V1CheckId::BudgetExceeded => "budget-exceeded",
        }
    }
}

impl fmt::Display for V1CheckId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One machine-readable v1 finding (issue #46): what check fired, on what,
/// with the evidence and the normative citation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct V1Finding {
    pub severity: DoctorSeverity,
    /// Which check fired. Serializes to the same kebab-case token it always
    /// has; it is a type so a typo is a compile error rather than a finding
    /// nothing matches (#347).
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

/// What a listen window observed (#161) — the scope statement that keeps
/// its findings honest (O5: `**` never crosses an `@`-chunk, so this names
/// exactly which selectors were watched), and the drop count that taints
/// them (O6).
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
    /// Key projections the bounded facts cache (#107) retired during the
    /// window — non-zero means `keys_seen` and the budget sweep cover the
    /// retained keys only (RFC 09 §5.1 O6). Absent when zero.
    #[serde(skip_serializing_if = "u64_is_zero", default)]
    pub facts_evicted: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The observation is a wire fact on `check conform`'s report: pinned,
    /// with the eviction count absent at zero and present by name when the
    /// bound cost keys.
    #[test]
    fn an_observation_summary_is_pinned() {
        let obs = ObservationSummary {
            window_s: 10.0,
            scopes: vec!["v1/*/state/**".into()],
            samples: 42,
            keys_seen: 7,
            dropped: 1,
            synthetic_marked: 3,
            facts_evicted: 0,
        };
        assert_eq!(
            serde_json::to_value(&obs).unwrap(),
            serde_json::json!({
                "window_s": 10.0,
                "scopes": ["v1/*/state/**"],
                "samples": 42,
                "keys_seen": 7,
                "dropped": 1,
                "synthetic_marked": 3,
            })
        );
        let evicted = ObservationSummary {
            facts_evicted: 5,
            ..obs
        };
        assert_eq!(serde_json::to_value(&evicted).unwrap()["facts_evicted"], 5);
    }

    /// The id vocabulary is API for the verbs that read it: if this test
    /// fails you are renaming a check id — don't.
    #[test]
    fn check_ids_are_stable() {
        assert_eq!(
            V1CheckId::ALL.map(V1CheckId::as_str),
            [
                "slice-parse",
                "slice-sync",
                "describe-totality",
                "schema-drift",
                "stale-state",
                "payload-undecodable",
                "payload-invalid",
                "qos-observed-mismatch",
                "unregistered-traffic",
                "rate-over-declared",
                "cardinality-over-declared",
                "kind-mismatch",
                "budget-exceeded",
            ]
        );
        for id in V1CheckId::ALL {
            let json = serde_json::to_string(&id).unwrap();
            assert_eq!(json, format!("\"{}\"", id.as_str()));
            assert_eq!(serde_json::from_str::<V1CheckId>(&json).unwrap(), id);
        }
    }
}

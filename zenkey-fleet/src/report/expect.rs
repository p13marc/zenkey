//! The expectation plane: a CI-facing assertion over a window, its
//! violations, and the verdict that follows (#160).

use serde::Serialize;

/// The `zenctl expect` verdict (#160). Three states, exit-coded 0/1/2: a CI
/// assertion that cannot tell "condition not met" from "I could not observe
/// properly" violates O4/O6 exactly where nobody reads logs carefully.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectVerdict {
    /// The expectation held within the window.
    Met,
    /// It did not, on a clean observation: conclusive positive evidence, or
    /// a shortfall counted with zero drops.
    NotMet,
    /// The observation cannot carry the claim (drops under a completeness
    /// claim, or a shortfall the dropped samples could have filled).
    Impaired,
}

impl ExpectVerdict {
    /// The [`Judgement`](crate::report::Judgement) mapping (RFC 13,
    /// v1.24). The judged claim is the finding — "the expectation was
    /// violated" — so `Met` is the established-**clean** pole:
    ///
    /// | verdict | judgement | exit (RFC 13 = this family's own contract) |
    /// |---|---|---|
    /// | `NotMet` | `Established` (finding) | 1 |
    /// | `Met` | `NotEstablished` (clean) | 0 |
    /// | `Impaired` | `Unobservable` | 2 |
    pub fn to_judgement(self) -> crate::report::Judgement {
        use crate::report::Judgement;
        match self {
            ExpectVerdict::NotMet => Judgement::Established,
            ExpectVerdict::Met => Judgement::NotEstablished {
                reason: "the expectation held within the window".into(),
            },
            ExpectVerdict::Impaired => Judgement::Unobservable {
                reason: "the observation cannot carry the claim (RFC 09 §5.1 O6)".into(),
            },
        }
    }
}

/// The inverse of [`ExpectVerdict::to_judgement`] — what lets `expect` fold
/// a judge's answer straight into its verdict without hand-mapping. Both
/// unestablished poles are `Impaired`: an assertion that was not (or could
/// not be) observed is not met and not violated.
impl From<crate::report::Judgement> for ExpectVerdict {
    fn from(j: crate::report::Judgement) -> ExpectVerdict {
        use crate::report::Judgement;
        match j {
            Judgement::Established => ExpectVerdict::NotMet,
            Judgement::NotEstablished { .. } => ExpectVerdict::Met,
            Judgement::NotAsked | Judgement::Unobservable { .. } => ExpectVerdict::Impaired,
        }
    }
}

/// The `zenctl expect` report (#160) — the window, what rode through it,
/// and the judgement with its reasons spelled out.
#[derive(Debug, Clone, Serialize)]
pub struct ExpectReport {
    pub selector: String,
    /// The window actually observed (shorter than requested on early
    /// success).
    pub window_s: f64,
    pub ended_early: bool,
    pub samples: u64,
    pub keys_seen: usize,
    /// Samples the bounded observer missed (O6) — the reason `Impaired`
    /// exists.
    pub dropped: u64,
    /// Present only when a rate bound was requested; measured over the full
    /// requested window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_hz: Option<f64>,
    /// Up to a cap of per-sample failures, verbatim.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub violations: Vec<String>,
    /// The exact total behind the capped examples.
    pub violations_total: u64,
    /// Why the verdict is not `Met`, one sentence per failed requirement.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unmet: Vec<String>,
    pub verdict: ExpectVerdict,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same contract as the doctor pin: `zenctl expect --format json` is
    /// consumed by CI scripts, so the shape changes only deliberately (#160).
    #[test]
    fn expect_report_json_shape_is_pinned() {
        let report = ExpectReport {
            selector: "v1/*/state/sysinfo/health".into(),
            window_s: 2.5,
            ended_early: true,
            samples: 3,
            keys_seen: 2,
            dropped: 0,
            rate_hz: None,
            violations: vec![],
            violations_total: 0,
            unmet: vec![],
            verdict: ExpectVerdict::Met,
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "selector": "v1/*/state/sysinfo/health",
                "window_s": 2.5,
                "ended_early": true,
                "samples": 3,
                "keys_seen": 2,
                "dropped": 0,
                "violations_total": 0,
                "verdict": "met",
            })
        );
        // The optional fields appear when they carry facts — and `unmet`
        // spells out every failed requirement.
        let report = ExpectReport {
            rate_hz: Some(0.5),
            violations: vec!["k: invalid — /x: 3 is not a string".into()],
            violations_total: 1,
            unmet: vec!["1 sample(s) violated a per-sample requirement".into()],
            verdict: ExpectVerdict::NotMet,
            ended_early: false,
            ..report
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["rate_hz"], 0.5);
        assert_eq!(json["verdict"], "not_met");
        assert_eq!(json["violations"][0], "k: invalid — /x: 3 is not a string");
    }
}

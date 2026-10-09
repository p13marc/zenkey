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

/// The `zenctl check expect` report (#160; zk2's since #612, FJ8b) — the
/// address and resource watched, the window, what rode through it, and the
/// judgement with its reasons spelled out.
#[derive(Debug, Clone, Serialize)]
pub struct ExpectReport {
    /// The address or pattern, `<system>/<service>`.
    pub address: String,
    /// `<name>.v<major>`.
    pub iface: String,
    /// The revision samples were decoded against.
    pub fingerprint: String,
    /// `<kind token>/<template>`; absent when only presence was asked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    /// The key expressions subscribed, base-relative: what the window could
    /// see, and no wider. Empty when only presence was asked.
    pub selectors: Vec<String>,
    /// The window actually observed (shorter than requested on early
    /// success).
    pub window_s: f64,
    pub ended_early: bool,
    pub samples: u64,
    pub keys_seen: usize,
    /// Samples the bounded observer missed (O6) — the reason `Impaired`
    /// exists.
    pub dropped: u64,
    /// Samples put on a wildcard key and discarded by rule (R6). Not
    /// losses, and no claim needs them.
    pub discarded: u64,
    /// Samples on a concrete key that resolved to no member of the
    /// resource (#671): the bus and the contract disagree.
    pub unresolved: u64,
    /// Present only when a rate bound was requested; measured over the full
    /// requested window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_hz: Option<f64>,
    /// The presence read, when `--present` asked it (§8.1). Absent when not
    /// asked (O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence: Option<ExpectPresence>,
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

/// What an expectation's presence read found (§8.1): who holds the
/// interface's token, and whether the read could carry "nobody".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExpectPresence {
    /// The liveliness selector, base-relative.
    pub selector: String,
    /// The addresses holding the token, sorted.
    pub holders: Vec<String>,
    /// `false` when the read ended at its timeout: a holder it did not see
    /// may be there. A read access control refused is complete and empty.
    pub complete: bool,
    /// Why the read could not be made, when it could not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The `zenctl check probe` report (#59; zk2's since #612, FJ8b): one
/// resource read the way a consumer reads it, and whether values arrived
/// within the window.
#[derive(Debug, Clone, Serialize)]
pub struct ProbeReport {
    /// The address or pattern bound, `<system>/<service>`.
    pub address: String,
    /// `<name>.v<major>`.
    pub iface: String,
    /// The revision values were decoded against.
    pub fingerprint: String,
    /// `<kind token>/<template>`.
    pub resource: String,
    /// The key expressions subscribed, base-relative.
    pub selectors: Vec<String>,
    /// The window asked, seconds.
    pub window_s: f64,
    /// How long the probe waited: shorter when a value arrived early.
    pub elapsed_s: f64,
    /// A state resource's current state, read from the owner first (S4), as
    /// a consumer of state reads it. Absent for a stream or event resource,
    /// which has none to read (O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<ProbeCurrent>,
    /// Values the subscription delivered within the window.
    pub received: u64,
    /// Of those, values a consumer can use: decoded as their declared type
    /// (a raw type as its bytes), or a deletion.
    pub conforming: u64,
    /// Values that did not decode as their declared type, or failed it.
    pub nonconforming: u64,
    pub keys_seen: usize,
    /// Values that arrived past this tool's buffer (O6).
    pub lagged: u64,
    /// Samples on a wildcard key, discarded by rule (R6). Not values.
    pub discarded: u64,
    /// Samples on a key that resolved to no member of the resource (#671).
    pub unresolved: u64,
    /// The first value, as the consumer received it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first: Option<Box<super::WatchSample>>,
    /// Who holds the interface's token, read only when nothing arrived:
    /// what makes the silence attributable (the tooling guide's §2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence: Option<ExpectPresence>,
    /// The judgement: the question "do values arrive within the window?"
    /// has its finding on *no* — `established` is a probe that failed.
    pub verdict: super::Judgement,
}

/// A probe's read of an owner's current state (S4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProbeCurrent {
    /// Keys the owner answered for, values and deletions.
    pub answered: usize,
    /// Of those, values a consumer can use.
    pub conforming: usize,
    /// Why the GET could not be made, when it could not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same contract as the doctor pin: `zenctl check expect --format json`
    /// is consumed by CI scripts, so the shape changes only deliberately
    /// (#160).
    #[test]
    fn expect_report_json_shape_is_pinned() {
        let report = ExpectReport {
            address: "host-a/tc".into(),
            iface: "tc.netif.v1".into(),
            fingerprint: format!("sha256:{}", "ab".repeat(32)),
            resource: Some("stream/bandwidth/{ns}/{iface}".into()),
            selectors: vec!["zk2/host-a/tc/tc.netif.v1/stream/bandwidth/*/*".into()],
            window_s: 2.5,
            ended_early: true,
            samples: 3,
            keys_seen: 2,
            dropped: 0,
            discarded: 0,
            unresolved: 0,
            rate_hz: None,
            presence: None,
            violations: vec![],
            violations_total: 0,
            unmet: vec![],
            verdict: ExpectVerdict::Met,
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "address": "host-a/tc",
                "iface": "tc.netif.v1",
                "fingerprint": format!("sha256:{}", "ab".repeat(32)),
                "resource": "stream/bandwidth/{ns}/{iface}",
                "selectors": ["zk2/host-a/tc/tc.netif.v1/stream/bandwidth/*/*"],
                "window_s": 2.5,
                "ended_early": true,
                "samples": 3,
                "keys_seen": 2,
                "dropped": 0,
                "discarded": 0,
                "unresolved": 0,
                "violations_total": 0,
                "verdict": "met",
            })
        );
        // The optional fields appear when they carry facts — and `unmet`
        // spells out every failed requirement.
        let report = ExpectReport {
            rate_hz: Some(0.5),
            presence: Some(ExpectPresence {
                selector: "zk2/host-a/tc/@zk/alive/tc.netif.v1/**".into(),
                holders: vec![],
                complete: true,
                error: None,
            }),
            violations: vec!["k: invalid — /x: expected string".into()],
            violations_total: 1,
            unmet: vec!["1 sample(s) violated a per-sample requirement".into()],
            verdict: ExpectVerdict::NotMet,
            ended_early: false,
            ..report
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["rate_hz"], 0.5);
        assert_eq!(json["verdict"], "not_met");
        assert_eq!(json["presence"]["holders"], serde_json::json!([]));
        assert_eq!(json["presence"]["complete"], true);
        assert_eq!(json["violations"][0], "k: invalid — /x: expected string");
    }

    /// The probe's shape: a stream resource asks no current state (absent,
    /// not zero), presence rides only when nothing arrived, and the verdict
    /// is the judgement core spelled as it always is.
    #[test]
    fn probe_report_json_shape_is_pinned() {
        let report = ProbeReport {
            address: "host-a/tc".into(),
            iface: "tc.netif.v1".into(),
            fingerprint: format!("sha256:{}", "ab".repeat(32)),
            resource: "stream/bandwidth/{ns}/{iface}".into(),
            selectors: vec!["zk2/host-a/tc/tc.netif.v1/stream/bandwidth/*/*".into()],
            window_s: 5.0,
            elapsed_s: 5.0,
            current: None,
            received: 0,
            conforming: 0,
            nonconforming: 0,
            keys_seen: 0,
            lagged: 0,
            discarded: 0,
            unresolved: 0,
            first: None,
            presence: Some(ExpectPresence {
                selector: "zk2/host-a/tc/@zk/alive/tc.netif.v1/**".into(),
                holders: vec!["host-a/tc".into()],
                complete: true,
                error: None,
            }),
            verdict: crate::report::Judgement::Established,
        };
        let json = serde_json::to_value(&report).unwrap();
        assert!(json.get("current").is_none(), "{json}");
        assert!(json.get("first").is_none(), "{json}");
        assert_eq!(
            json["presence"]["holders"],
            serde_json::json!(["host-a/tc"])
        );
        assert_eq!(
            json["verdict"],
            serde_json::json!({"answer": "established"})
        );
        assert_eq!(json["received"], 0, "a count, present at zero");
    }
}

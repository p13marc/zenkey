//! zk2's `check conform` (#703): one service against the contract
//! revision it claims, as a conformance suite — one verdict per case.
//!
//! **Polarity** (`docs/zk2/tooling-guide.md` §1). Every case asks "does the
//! service break this rule here?", so a case's finding is the *yes*:
//! `Established`, with what broke it in `detail`. A case the service passes
//! is `NotEstablished`, with the evidence as its reason; one whose
//! observation could not be had is `Unobservable` — a resource nothing was
//! heard from in the window, a stamp no `meta.zid` attributes — and one the
//! run did not ask is `NotAsked`, with why in `detail`: an operation that
//! is not idempotent without `--i-know`, a raw type with no structure, a
//! resource that declares no `freshness.ttl_s`, a profile-backed case whose
//! profile does not exist yet (#613). The `freshness` case is the "no" of
//! `freshness.v1`'s "is this value fresh?" turned into the suite's
//! polarity: a stale member is the finding (#720).

use std::fmt;

use serde::Serialize;

use super::judgement::Judgement;

/// The cases of the suite. **Stable API**: scripts and JUnit consumers key
/// on these; new cases append.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaseId {
    /// The bundle the descriptor names is served by a holder and verifies
    /// (§8.2, §8.4): an owner holds the bundle of every interface it
    /// implements.
    ContractServed,
    /// An exposed stream, state or event resource is served: a sample in
    /// the window, or a state reply (§8.2, §3.3).
    ResourceServed,
    /// Every sample and reply of the resource decodes as its declared type
    /// and, for a JSON Schema type, satisfies it (§7.2, §7.3).
    PayloadType,
    /// Every sample rode the resource's declared QoS: priority, congestion
    /// control, express (§2.4).
    Qos,
    /// An operation answers as O1–O7 say: a value on its own concrete key
    /// or a valid envelope, never silence (O3); exactly one summary per
    /// replier when one is declared (O6); a response of its type (§7.2).
    Operation,
    /// A call over a template's wildcard to an operation that forbids
    /// fan-out is refused `fanout_forbidden` before any handler runs (O2).
    FanoutRefused,
    /// Every state mutation and reply carries the owner's own stamp, its
    /// clock the descriptor's `meta.zid`, compared by value (S1, 0.11).
    StateStamp,
    /// The owner answers a GET over the state resource, each reply carrying
    /// its mutation's timestamp (S2).
    StateGet,
    /// Each member of a stream or state resource that declares
    /// `freshness.ttl_s` confirmed within that horizon, at the window's end
    /// (`freshness.v1` §5, #720): by this run's receive clock, or a GET
    /// reply's stamp against a clock trusted to the HLC delta. One verdict
    /// per resource; a resource with no horizon is not asked.
    Freshness,
    /// Rates and populations within a declared budget: a profile's, which
    /// waits for it (#613). Never asked here.
    Budget,
}

impl CaseId {
    /// Every case, in the order the suite reports them.
    pub const ALL: [CaseId; 10] = [
        CaseId::ContractServed,
        CaseId::ResourceServed,
        CaseId::PayloadType,
        CaseId::Qos,
        CaseId::Operation,
        CaseId::FanoutRefused,
        CaseId::StateStamp,
        CaseId::StateGet,
        CaseId::Freshness,
        CaseId::Budget,
    ];

    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            CaseId::ContractServed => "contract-served",
            CaseId::ResourceServed => "resource-served",
            CaseId::PayloadType => "payload-type",
            CaseId::Qos => "qos",
            CaseId::Operation => "operation",
            CaseId::FanoutRefused => "fanout-refused",
            CaseId::StateStamp => "state-stamp",
            CaseId::StateGet => "state-get",
            CaseId::Freshness => "freshness",
            CaseId::Budget => "budget",
        }
    }

    /// The core section (or profile) the case reads.
    pub fn section(self) -> &'static str {
        match self {
            CaseId::ContractServed => "§8.4",
            CaseId::ResourceServed => "§8.2",
            CaseId::PayloadType => "§7.2",
            CaseId::Qos => "§2.4",
            CaseId::Operation => "§5.1",
            CaseId::FanoutRefused => "§5.1 O2",
            CaseId::StateStamp => "§4.2 S1",
            CaseId::StateGet => "§4.2 S2",
            CaseId::Freshness => "freshness.v1",
            CaseId::Budget => "#613",
        }
    }
}

impl fmt::Display for CaseId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One case's verdict, on one subject.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConformCase {
    pub case: CaseId,
    /// What the case judged: a resource (`<kind token>/<template>`), the
    /// revision, or `service`.
    pub subject: String,
    pub section: &'static str,
    /// The case's question answered: a violation is the yes.
    pub verdict: Judgement,
    /// With `established`, what broke the rule; with `not_asked`, why the
    /// question was not put. Absent otherwise: the other poles carry their
    /// reason in the verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ConformCase {
    pub fn passed(case: CaseId, subject: impl Into<String>, evidence: impl Into<String>) -> Self {
        ConformCase {
            case,
            subject: subject.into(),
            section: case.section(),
            verdict: Judgement::NotEstablished {
                reason: evidence.into(),
            },
            detail: None,
        }
    }

    pub fn failed(case: CaseId, subject: impl Into<String>, finding: impl Into<String>) -> Self {
        ConformCase {
            case,
            subject: subject.into(),
            section: case.section(),
            verdict: Judgement::Established,
            detail: Some(finding.into()),
        }
    }

    pub fn unobservable(case: CaseId, subject: impl Into<String>, why: impl Into<String>) -> Self {
        ConformCase {
            case,
            subject: subject.into(),
            section: case.section(),
            verdict: Judgement::Unobservable { reason: why.into() },
            detail: None,
        }
    }

    pub fn not_asked(case: CaseId, subject: impl Into<String>, why: impl Into<String>) -> Self {
        ConformCase {
            case,
            subject: subject.into(),
            section: case.section(),
            verdict: Judgement::NotAsked,
            detail: Some(why.into()),
        }
    }
}

/// `zenctl check conform` (#703): one service, the revision it claims, and
/// every case.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConformReport {
    /// `<system>/<service>`.
    pub address: String,
    /// `<name>.v<major>`.
    pub iface: String,
    /// The revision under test: the one the service's descriptor claims.
    /// Absent when none could be had.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// The deployment's namespace; empty for the bus root.
    pub namespace: String,
    /// How long the data resources were listened to, seconds.
    pub window_s: f64,
    /// The selectors put to the bus, base-relative in the namespace: what
    /// the suite read, and no wider (tooling guide O5).
    pub asked: Vec<String>,
    pub cases: Vec<ConformCase>,
    /// Set when the service could not be judged at all — no token visible,
    /// no descriptor, the interface not implemented — with what was read.
    /// Every case is then unobservable, and the run is no verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unobservable: Option<String>,
}

impl ConformReport {
    /// Every case that found a violation.
    pub fn failures(&self) -> impl Iterator<Item = &ConformCase> {
        self.cases
            .iter()
            .filter(|c| c.verdict == Judgement::Established)
    }

    /// The run as one [`Judgement`]: a violation anywhere is the finding
    /// (exit 1); without one, a case left unobservable — or the service
    /// unobservable whole — could be hiding one (exit 2); a case not asked
    /// counts for neither, and a run that asked nothing is no verdict; every
    /// case asked and passed is clean (exit 0).
    pub fn judgement(&self) -> Judgement {
        let failed = self.failures().count();
        if failed > 0 {
            return Judgement::Established;
        }
        if let Some(why) = &self.unobservable {
            return Judgement::Unobservable {
                reason: why.clone(),
            };
        }
        let asked: Vec<&ConformCase> = self
            .cases
            .iter()
            .filter(|c| !c.verdict.is_not_asked())
            .collect();
        if asked.is_empty() {
            return Judgement::Unobservable {
                reason: "no case was asked".into(),
            };
        }
        let unseen: Vec<String> = asked
            .iter()
            .filter(|c| c.verdict.is_unobservable())
            .map(|c| format!("{} {}", c.case, c.subject))
            .collect();
        if !unseen.is_empty() {
            return Judgement::Unobservable {
                reason: format!(
                    "no violation, and {} case(s) could not be established: {}",
                    unseen.len(),
                    unseen.join(", ")
                ),
            };
        }
        Judgement::NotEstablished {
            reason: format!(
                "{} case(s) asked, every one passed against {}",
                asked.len(),
                self.fingerprint.as_deref().unwrap_or("the revision")
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::judgement_exit_code;
    use serde_json::json;

    /// The case vocabulary is API: additions append, nothing renames.
    #[test]
    fn case_ids_are_stable() {
        assert_eq!(
            CaseId::ALL.map(CaseId::as_str),
            [
                "contract-served",
                "resource-served",
                "payload-type",
                "qos",
                "operation",
                "fanout-refused",
                "state-stamp",
                "state-get",
                "freshness",
                "budget"
            ]
        );
        for c in CaseId::ALL {
            assert_eq!(serde_json::to_value(c).unwrap(), json!(c.as_str()));
            assert!(!c.section().is_empty());
        }
    }

    fn report(cases: Vec<ConformCase>) -> ConformReport {
        ConformReport {
            address: "host-a/tc".into(),
            iface: "tc.netif.v1".into(),
            fingerprint: Some(format!("sha256:{}", "ab".repeat(32))),
            namespace: "acme".into(),
            window_s: 5.0,
            asked: vec!["zk2/host-a/tc/@zk/**".into()],
            cases,
            unobservable: None,
        }
    }

    /// The report's wire shape: each pole with its own `answer`, `detail`
    /// on the established and the not-asked, absent elsewhere.
    #[test]
    fn the_conform_report_pins_its_shape() {
        let r = report(vec![
            ConformCase::passed(CaseId::Qos, "stream/bandwidth/{ns}/{iface}", "12 sample(s)"),
            ConformCase::failed(
                CaseId::PayloadType,
                "stream/bandwidth/{ns}/{iface}",
                "invalid",
            ),
            ConformCase::unobservable(CaseId::ResourceServed, "state/namespaces", "silent"),
            ConformCase::failed(
                CaseId::Freshness,
                "state/interfaces/{ns}/{iface}",
                "1 of 1 member(s) stale",
            ),
            ConformCase::not_asked(CaseId::Freshness, "state/namespaces", "no freshness.ttl_s"),
        ]);
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            json!({
                "address": "host-a/tc",
                "iface": "tc.netif.v1",
                "fingerprint": format!("sha256:{}", "ab".repeat(32)),
                "namespace": "acme",
                "window_s": 5.0,
                "asked": ["zk2/host-a/tc/@zk/**"],
                "cases": [
                    {"case": "qos", "subject": "stream/bandwidth/{ns}/{iface}", "section": "§2.4",
                     "verdict": {"answer": "not_established", "reason": "12 sample(s)"}},
                    {"case": "payload-type", "subject": "stream/bandwidth/{ns}/{iface}",
                     "section": "§7.2", "verdict": {"answer": "established"}, "detail": "invalid"},
                    {"case": "resource-served", "subject": "state/namespaces", "section": "§8.2",
                     "verdict": {"answer": "unobservable", "reason": "silent"}},
                    {"case": "freshness", "subject": "state/interfaces/{ns}/{iface}",
                     "section": "freshness.v1", "verdict": {"answer": "established"},
                     "detail": "1 of 1 member(s) stale"},
                    {"case": "freshness", "subject": "state/namespaces", "section": "freshness.v1",
                     "verdict": {"answer": "not_asked"}, "detail": "no freshness.ttl_s"},
                ],
            })
        );
    }

    /// The run's judgement: a violation is the 1; without one, an
    /// unobservable case or an unobservable service is the 2; every case
    /// asked and passed is the 0; not asked counts for neither.
    #[test]
    fn the_judgement_reads_failures_then_the_unobservable() {
        let pass = ConformCase::passed(CaseId::Qos, "s", "ok");
        let fail = ConformCase::failed(CaseId::Qos, "t", "bad");
        let unseen = ConformCase::unobservable(CaseId::ResourceServed, "u", "silent");
        let skip = ConformCase::not_asked(CaseId::Budget, "service", "#613");
        let exit = |r: &ConformReport| judgement_exit_code(&r.judgement());
        assert_eq!(exit(&report(vec![pass.clone(), skip.clone()])), 0);
        assert_eq!(exit(&report(vec![pass.clone(), fail, unseen.clone()])), 1);
        assert_eq!(exit(&report(vec![pass.clone(), unseen])), 2);
        assert_eq!(exit(&report(vec![skip])), 2, "nothing asked");
        let whole = ConformReport {
            unobservable: Some("no token visible to this reader".into()),
            ..report(vec![pass])
        };
        assert_eq!(exit(&whole), 2);
    }
}

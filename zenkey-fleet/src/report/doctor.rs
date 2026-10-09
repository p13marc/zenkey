//! zk2's doctor (#612, FJ6): one verdict per check, in the judgement shape.
//!
//! Every check asks one question about the deployment and answers it on the
//! RFC 13 core ([`Judgement`]), with the polarity the tooling guide asks
//! every vocabulary to document (`docs/zk2/tooling-guide.md` §1): each id
//! is **named for a condition firing**, so its finding is the *yes* —
//! `Established` — and a clean check is `NotEstablished`, with the evidence
//! that makes it clean as its reason. A check whose input could not be had
//! is `Unobservable` with the reason, never clean; one the run was told not
//! to ask is `NotAsked`. The two Unestablished poles stay apart in every
//! medium, as everywhere else in this crate.
//!
//! A check carries three lists, each spelled once:
//! - **findings**: what fired, one per subject, with its severity — the
//!   subject is the key [`crate::doctor_delta`] compares runs on;
//! - **unjudged**: the subjects it could not decide, each with what stood
//!   in the way (a descriptor that did not answer, a bundle no holder
//!   served, a read that ended at its timeout);
//! - **section**: the core section it enforces.
//!
//! [`DoctorScope`] is what makes an empty findings list legible: what the
//! run read, in which namespace, and whether its reads were complete. An
//! empty scope (no zk2 token visible to this reader) is the report's own
//! [`DoctorReport::unobservable`]: a doctor pointed at the wrong namespace
//! must not be green.

use std::fmt;

use super::asked::Asked;
use super::judgement::Judgement;
use serde::{Deserialize, Serialize};

/// Every check zk2's doctor runs.
///
/// **Stable API**: scripts key on these through `--format json`, a
/// watchdog's `doctor <CHECK-ID>` rule names one, and a notifier keys its
/// deltas on them. New checks append; nothing renames one.
/// [`CheckId::question`] is each one's question, worded so that its finding
/// is the yes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckId {
    /// Two instances of one service exposing an exclusive resource at once
    /// (§6), from two presence reads a grace period apart.
    SplitBrain,
    /// A role whose bindings select no provider this reader can see (§3.2).
    BindingUnsatisfied,
    /// Providers of one interface at revisions the classifier calls review
    /// or breaking against each other (§9.8).
    ContractDrift,
    /// A revision a descriptor names that no holder serves verified (§8.4).
    ContractUnavailable,
    /// A descriptor that fails the descriptor check: the D-codes (§3.3).
    DescriptorInvalid,
    /// An instance whose tokens disagree with its descriptor (§8.1).
    TokenMissing,
    /// More tokens in the presence domain than its budget (§8.3).
    PresenceOverBudget,
    /// A router storage answering on an owner's state keys (§4.2 S4).
    StorageOnState,
    /// An archive serving keys its alignment has not confirmed (§4.4).
    ArchiveUnaligned,
    /// An owner answering its state with a stamp that is not its own
    /// (§4.2 S1–S2). A data-plane read, asked under `deep` only.
    StateStampForeign,
    /// This host's `RLIMIT_MEMLOCK` below what a shared-memory pool needs
    /// (§7.4).
    ShmMemlockLow,
    /// No router answering the admin space, which S4's check reads (§4.2).
    AdminUnreachable,
    /// Routers of the mesh at different zenoh versions (Appendix B).
    RouterVersionSkew,
}

impl CheckId {
    /// Every check id, in the order the doctor reports them.
    pub const ALL: [CheckId; 13] = [
        CheckId::SplitBrain,
        CheckId::BindingUnsatisfied,
        CheckId::ContractDrift,
        CheckId::ContractUnavailable,
        CheckId::DescriptorInvalid,
        CheckId::TokenMissing,
        CheckId::PresenceOverBudget,
        CheckId::StorageOnState,
        CheckId::ArchiveUnaligned,
        CheckId::StateStampForeign,
        CheckId::ShmMemlockLow,
        CheckId::AdminUnreachable,
        CheckId::RouterVersionSkew,
    ];

    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            CheckId::SplitBrain => "split-brain",
            CheckId::BindingUnsatisfied => "binding-unsatisfied",
            CheckId::ContractDrift => "contract-drift",
            CheckId::ContractUnavailable => "contract-unavailable",
            CheckId::DescriptorInvalid => "descriptor-invalid",
            CheckId::TokenMissing => "token-missing",
            CheckId::PresenceOverBudget => "presence-over-budget",
            CheckId::StorageOnState => "storage-on-state",
            CheckId::ArchiveUnaligned => "archive-unaligned",
            CheckId::StateStampForeign => "state-stamp-foreign",
            CheckId::ShmMemlockLow => "shm-memlock-low",
            CheckId::AdminUnreachable => "admin-unreachable",
            CheckId::RouterVersionSkew => "router-version-skew",
        }
    }

    /// Read a check id a caller supplied: `doctor --check`, a watchdog's
    /// `doctor <CHECK-ID>` rule, a script's filter.
    pub fn parse(token: &str) -> Option<CheckId> {
        CheckId::ALL.into_iter().find(|c| c.as_str() == token)
    }

    /// The core section the check enforces (`spec/core.md`).
    pub fn section(self) -> &'static str {
        match self {
            CheckId::SplitBrain => "§6",
            CheckId::BindingUnsatisfied => "§3.2 R5",
            CheckId::ContractDrift => "§9.8",
            CheckId::ContractUnavailable => "§8.4",
            CheckId::DescriptorInvalid => "§3.3",
            CheckId::TokenMissing => "§8.1",
            CheckId::PresenceOverBudget => "§8.3",
            CheckId::StorageOnState => "§4.2 S4",
            CheckId::ArchiveUnaligned => "§4.4",
            CheckId::StateStampForeign => "§4.2 S1–S2",
            CheckId::ShmMemlockLow => "§7.4",
            CheckId::AdminUnreachable => "§4.2",
            CheckId::RouterVersionSkew => "App. B",
        }
    }

    /// The check's question, worded so that its finding is the **yes**:
    /// the polarity every renderer and exit map reads (tooling guide §1).
    pub fn question(self) -> &'static str {
        match self {
            CheckId::SplitBrain => {
                "do two instances of one service hold one interface's token for longer than \
                 the grace period, with at least two exposing an exclusive resource?"
            }
            CheckId::BindingUnsatisfied => {
                "does a role's binding select no provider visible to this reader?"
            }
            CheckId::ContractDrift => {
                "do providers of one interface serve revisions the classifier calls review \
                 or breaking against each other?"
            }
            CheckId::ContractUnavailable => {
                "does a descriptor name a revision that no holder serves verified?"
            }
            CheckId::DescriptorInvalid => "does a descriptor fail the descriptor check?",
            CheckId::TokenMissing => {
                "do an instance's tokens disagree with its descriptor: an exposed interface \
                 outside the tokenless set without its token, or a token its descriptor \
                 does not list?"
            }
            CheckId::PresenceOverBudget => {
                "does the presence domain hold more tokens than its budget?"
            }
            CheckId::StorageOnState => "does a router storage answer on an owner's state keys?",
            CheckId::ArchiveUnaligned => {
                "does an archive serve keys its alignment has not confirmed?"
            }
            CheckId::StateStampForeign => {
                "does an owner answer its state with a stamp that is not its own?"
            }
            CheckId::ShmMemlockLow => {
                "is this host's RLIMIT_MEMLOCK below what a shared-memory pool needs?"
            }
            CheckId::AdminUnreachable => "does no router answer the admin space?",
            CheckId::RouterVersionSkew => "do the routers run different zenoh versions?",
        }
    }

    /// Whether the check reads the deployment's presence. Over an empty
    /// scope (no zk2 token visible) these are unobservable; the others —
    /// the admin space, the presence domain, this host — are not about the
    /// namespace and still answer.
    pub fn reads_presence(self) -> bool {
        !matches!(
            self,
            CheckId::PresenceOverBudget
                | CheckId::StorageOnState
                | CheckId::ShmMemlockLow
                | CheckId::AdminUnreachable
                | CheckId::RouterVersionSkew
        )
    }
}

impl fmt::Display for CheckId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How bad a finding is: the ladder a `--fail-on` floor reads.
///
/// Shared with v1's check vocabulary ([`crate::report::V1Finding`]), whose
/// findings `field` and `check conform` still produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorSeverity {
    /// A rule broken: a MUST or MUST NOT of the core, or the deployment
    /// disagreeing with itself.
    Error,
    /// A SHOULD unmet, or a change a human has to accept: judgement is
    /// degraded, not wrong.
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

/// One finding: what fired, on what, and the evidence.
///
/// `(check, subject)` is the finding's identity: [`crate::doctor_delta`]
/// compares runs on it, so a subject is spelled the same way every run —
/// `<system>/<service>@<instance>`, `<system>/<service> <role>`,
/// `<iface> <fingerprint>`, `<storage>@<zid>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorFinding {
    pub severity: DoctorSeverity,
    pub check: CheckId,
    pub subject: String,
    /// What was observed, for a person.
    pub evidence: String,
}

/// A subject a check could not decide, and what stood in the way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unjudged {
    pub subject: String,
    pub reason: String,
}

/// One check's verdict (tooling guide §1): `Established` when it found
/// something, `NotEstablished` with the evidence of a clean answer,
/// `Unobservable` when a subject it had to decide could not be read, and
/// `NotAsked` when the run was not to ask it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckReport {
    pub check: CheckId,
    /// The core section the check enforces.
    pub section: &'static str,
    /// The check's question answered: the finding is the yes.
    pub verdict: Judgement,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<DoctorFinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unjudged: Vec<Unjudged>,
}

impl CheckReport {
    /// A check the run did not ask.
    pub fn not_asked(check: CheckId) -> CheckReport {
        CheckReport {
            check,
            section: check.section(),
            verdict: Judgement::NotAsked,
            findings: Vec::new(),
            unjudged: Vec::new(),
        }
    }

    /// A check whose input could not be had at all.
    pub fn unobservable(check: CheckId, reason: impl Into<String>) -> CheckReport {
        CheckReport {
            check,
            section: check.section(),
            verdict: Judgement::Unobservable {
                reason: reason.into(),
            },
            findings: Vec::new(),
            unjudged: Vec::new(),
        }
    }

    /// The verdict its lists support: a finding is `Established`; with
    /// none, a subject left undecided is `Unobservable`; with neither, the
    /// check is clean and `clean` says why.
    pub fn of(
        check: CheckId,
        findings: Vec<DoctorFinding>,
        unjudged: Vec<Unjudged>,
        clean: impl Into<String>,
    ) -> CheckReport {
        let verdict = if !findings.is_empty() {
            Judgement::Established
        } else if let [one] = unjudged.as_slice() {
            Judgement::Unobservable {
                reason: format!("{}: {}", one.subject, one.reason),
            }
        } else if let Some(first) = unjudged.first() {
            Judgement::Unobservable {
                reason: format!(
                    "{} subjects could not be judged; the first, {}: {}",
                    unjudged.len(),
                    first.subject,
                    first.reason
                ),
            }
        } else {
            Judgement::NotEstablished {
                reason: clean.into(),
            }
        };
        CheckReport {
            check,
            section: check.section(),
            verdict,
            findings,
            unjudged,
        }
    }
}

/// What one doctor run read: the scope that makes its silences legible.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DoctorScope {
    /// The deployment's namespace; empty for the bus root.
    pub namespace: String,
    /// The presence selector, base-relative in the namespace. Absent when
    /// no check that reads presence was asked.
    #[serde(default, skip_serializing_if = "Asked::is_not_asked")]
    pub presence: Asked<DoctorPresence>,
    /// Routers that answered the admin space (`@/*/router`), read in no
    /// namespace. Absent when no check that reads it was asked.
    #[serde(default, skip_serializing_if = "Asked::is_not_asked")]
    pub routers: Asked<usize>,
}

/// The presence half of [`DoctorScope`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DoctorPresence {
    pub selector: String,
    /// Whether every presence read ended at the routers' final reply. A
    /// read that ended at its timeout may have missed tokens (§8.1).
    pub complete: bool,
    /// How far apart the two reads split-brain compares were taken.
    pub grace_s: f64,
    pub services: usize,
    pub instances: usize,
    pub tokens: usize,
    /// Instances whose descriptor was asked for and did not read.
    pub undescribed: usize,
    /// Revisions the descriptors name, and how many of them are held.
    pub revisions: usize,
    pub held: usize,
}

/// zk2's doctor report: one [`CheckReport`] per check, in
/// [`CheckId::ALL`] order, and the scope they were judged over.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DoctorReport {
    pub scope: DoctorScope,
    pub checks: Vec<CheckReport>,
    /// Set when presence showed no zk2 token at all — the empty scope —
    /// with what was read. Every check that reads presence is then
    /// unobservable, and the run is no verdict however clean the rest:
    /// a doctor pointed at the wrong namespace must not be green.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unobservable: Option<String>,
}

impl DoctorReport {
    /// One check's verdict.
    pub fn check(&self, id: CheckId) -> Option<&CheckReport> {
        self.checks.iter().find(|c| c.check == id)
    }

    /// Every finding of every check, in check order.
    pub fn findings(&self) -> impl Iterator<Item = &DoctorFinding> {
        self.checks.iter().flat_map(|c| c.findings.iter())
    }

    /// How many findings have `severity`.
    pub fn count(&self, severity: DoctorSeverity) -> usize {
        self.findings().filter(|f| f.severity == severity).count()
    }

    /// The run as one [`Judgement`] under a severity `floor` — what a
    /// frontend exits through.
    ///
    /// The judged claim is *"the deployment has a finding at or above
    /// `floor`"*: a hit is `Established` (exit 1). Without one, the run is
    /// clean only when every check it asked was established clean: an
    /// empty scope, or a check left unobservable, could be hiding the
    /// finding, so the run is `Unobservable` (exit 2). A check not asked
    /// is not counted either way, and a run that asked nothing is no
    /// verdict.
    pub fn judgement(&self, floor: DoctorSeverity) -> Judgement {
        if self.findings().any(|f| f.severity.reaches(floor)) {
            return Judgement::Established;
        }
        if let Some(why) = &self.unobservable {
            return Judgement::Unobservable {
                reason: why.clone(),
            };
        }
        let asked: Vec<&CheckReport> = self
            .checks
            .iter()
            .filter(|c| !c.verdict.is_not_asked())
            .collect();
        if asked.is_empty() {
            return Judgement::Unobservable {
                reason: "no check was asked".into(),
            };
        }
        let unobservable: Vec<&str> = asked
            .iter()
            .filter(|c| c.verdict.is_unobservable())
            .map(|c| c.check.as_str())
            .collect();
        if !unobservable.is_empty() {
            return Judgement::Unobservable {
                reason: format!(
                    "no finding at or above {}, and {} check(s) could not be established: {}",
                    floor.as_str(),
                    unobservable.len(),
                    unobservable.join(", ")
                ),
            };
        }
        Judgement::NotEstablished {
            reason: format!(
                "no finding at or above {} among the {} check(s) asked",
                floor.as_str(),
                asked.len()
            ),
        }
    }
}

/// What one doctor run says relative to the previous one (#389): findings
/// keyed on `(check, subject)`, so evidence and severity drift count as
/// unchanged. Computed by [`crate::doctor_delta`]; routed by a notifier's
/// scheduled doctor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DoctorDelta {
    /// Findings present now and absent from the previous run.
    pub new: Vec<DoctorFinding>,
    /// Findings present in the previous run and gone now.
    pub fixed: Vec<DoctorFinding>,
    /// Findings present in both runs.
    pub unchanged: usize,
}

impl DoctorDelta {
    /// Whether `f` is one of the new findings, by its key.
    pub fn is_new(&self, f: &DoctorFinding) -> bool {
        self.new
            .iter()
            .any(|n| n.check == f.check && n.subject == f.subject)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::judgement_exit_code;

    fn finding(check: CheckId, severity: DoctorSeverity) -> DoctorFinding {
        DoctorFinding {
            severity,
            check,
            subject: "host-a/tc@3fa9c2d41b7e0012".into(),
            evidence: "e".into(),
        }
    }

    fn scope() -> DoctorScope {
        DoctorScope {
            namespace: "acme".into(),
            presence: Asked::Asked(DoctorPresence {
                selector: "zk2/*/*/@zk/**".into(),
                complete: true,
                grace_s: 2.0,
                services: 3,
                instances: 3,
                tokens: 7,
                undescribed: 0,
                revisions: 2,
                held: 2,
            }),
            routers: Asked::Asked(1),
        }
    }

    /// The serialized report is a wire contract: `zenctl doctor --format
    /// json` scripts and a notifier read this exact shape. Every verdict
    /// pole is pinned, the lists are absent when empty, and the empty scope
    /// is absent unless set.
    #[test]
    fn doctor_report_json_shape_is_pinned() {
        let report = DoctorReport {
            scope: scope(),
            checks: vec![
                CheckReport::of(
                    CheckId::SplitBrain,
                    vec![DoctorFinding {
                        severity: DoctorSeverity::Error,
                        check: CheckId::SplitBrain,
                        subject: "h1/tc tc.v1".into(),
                        evidence: "two holders".into(),
                    }],
                    vec![],
                    "unused",
                ),
                CheckReport::of(
                    CheckId::BindingUnsatisfied,
                    vec![],
                    vec![],
                    "every role bound",
                ),
                CheckReport::of(
                    CheckId::ContractDrift,
                    vec![],
                    vec![Unjudged {
                        subject: "tc.v1".into(),
                        reason: "a revision is unavailable".into(),
                    }],
                    "unused",
                ),
                CheckReport::not_asked(CheckId::StateStampForeign),
            ],
            unobservable: None,
        };
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::json!({
                "scope": {
                    "namespace": "acme",
                    "presence": {
                        "selector": "zk2/*/*/@zk/**",
                        "complete": true,
                        "grace_s": 2.0,
                        "services": 3,
                        "instances": 3,
                        "tokens": 7,
                        "undescribed": 0,
                        "revisions": 2,
                        "held": 2,
                    },
                    "routers": 1,
                },
                "checks": [
                    {
                        "check": "split-brain",
                        "section": "§6",
                        "verdict": {"answer": "established"},
                        "findings": [{
                            "severity": "error",
                            "check": "split-brain",
                            "subject": "h1/tc tc.v1",
                            "evidence": "two holders",
                        }],
                    },
                    {
                        "check": "binding-unsatisfied",
                        "section": "§3.2 R5",
                        "verdict": {"answer": "not_established", "reason": "every role bound"},
                    },
                    {
                        "check": "contract-drift",
                        "section": "§9.8",
                        "verdict": {
                            "answer": "unobservable",
                            "reason": "tc.v1: a revision is unavailable",
                        },
                        "unjudged": [{"subject": "tc.v1", "reason": "a revision is unavailable"}],
                    },
                    {
                        "check": "state-stamp-foreign",
                        "section": "§4.2 S1–S2",
                        "verdict": {"answer": "not_asked"},
                    },
                ],
            })
        );
        // The empty scope, by name and with its reason; and the scope's
        // halves absent when not asked, never zero.
        let empty = DoctorReport {
            scope: DoctorScope {
                namespace: String::new(),
                presence: Asked::NotAsked,
                routers: Asked::NotAsked,
            },
            checks: vec![],
            unobservable: Some("no zk2 token visible to this reader".into()),
        };
        assert_eq!(
            serde_json::to_value(&empty).unwrap(),
            serde_json::json!({
                "scope": {"namespace": ""},
                "checks": [],
                "unobservable": "no zk2 token visible to this reader",
            })
        );
    }

    /// The run's judgement: a finding at or above the floor is the 1; below
    /// it, a check left unobservable or an empty scope is the 2; every check
    /// asked clean is the 0; a check not asked counts for neither; and a
    /// run that asked nothing is no verdict.
    #[test]
    fn the_judgement_reads_the_floor_the_unobservable_checks_and_the_empty_scope() {
        let exit = |r: &DoctorReport, floor| judgement_exit_code(&r.judgement(floor));
        let clean = CheckReport::of(CheckId::SplitBrain, vec![], vec![], "none");
        let warned = CheckReport::of(
            CheckId::PresenceOverBudget,
            vec![finding(
                CheckId::PresenceOverBudget,
                DoctorSeverity::Warning,
            )],
            vec![],
            "unused",
        );
        let unseen = CheckReport::unobservable(CheckId::StorageOnState, "admin unreachable");
        let report = |checks: Vec<CheckReport>| DoctorReport {
            scope: scope(),
            checks,
            unobservable: None,
        };

        let r = report(vec![
            clean.clone(),
            CheckReport::not_asked(CheckId::TokenMissing),
        ]);
        assert_eq!(exit(&r, DoctorSeverity::Warning), 0);
        let r = report(vec![clean.clone(), warned.clone()]);
        assert_eq!(exit(&r, DoctorSeverity::Warning), 1);
        assert_eq!(
            exit(&r, DoctorSeverity::Error),
            0,
            "a warning under --fail-on error"
        );
        let r = report(vec![warned.clone(), unseen.clone()]);
        assert_eq!(
            exit(&r, DoctorSeverity::Warning),
            1,
            "a finding is a finding"
        );
        assert_eq!(
            exit(&r, DoctorSeverity::Error),
            2,
            "below the floor, an unobservable check could hide one"
        );
        let r = DoctorReport {
            unobservable: Some("no token".into()),
            ..report(vec![clean.clone()])
        };
        assert_eq!(exit(&r, DoctorSeverity::Warning), 2);
        let r = report(vec![CheckReport::not_asked(CheckId::SplitBrain)]);
        assert_eq!(exit(&r, DoctorSeverity::Warning), 2, "nothing asked");
    }

    /// The verdict a check's lists support: findings win, then the
    /// undecided, then the clean reason.
    #[test]
    fn a_check_s_verdict_follows_its_lists() {
        let f = finding(CheckId::TokenMissing, DoctorSeverity::Error);
        let u = Unjudged {
            subject: "s".into(),
            reason: "r".into(),
        };
        let both = CheckReport::of(CheckId::TokenMissing, vec![f], vec![u.clone()], "c");
        assert_eq!(both.verdict, Judgement::Established);
        let two = CheckReport::of(CheckId::TokenMissing, vec![], vec![u.clone(), u], "c");
        assert_eq!(
            two.verdict,
            Judgement::Unobservable {
                reason: "2 subjects could not be judged; the first, s: r".into()
            }
        );
        let clean = CheckReport::of(CheckId::TokenMissing, vec![], vec![], "c");
        assert_eq!(
            clean.verdict,
            Judgement::NotEstablished { reason: "c".into() }
        );
    }
}

#[cfg(test)]
mod check_id_tests {
    use super::*;

    /// The id vocabulary is API: additions append, nothing renames. If this
    /// test fails you are renaming a check id — don't.
    #[test]
    fn check_ids_are_stable() {
        assert_eq!(
            CheckId::ALL.map(CheckId::as_str),
            [
                "split-brain",
                "binding-unsatisfied",
                "contract-drift",
                "contract-unavailable",
                "descriptor-invalid",
                "token-missing",
                "presence-over-budget",
                "storage-on-state",
                "archive-unaligned",
                "state-stamp-foreign",
                "shm-memlock-low",
                "admin-unreachable",
                "router-version-skew",
            ]
        );
    }

    /// `as_str`, serde and `parse` are one vocabulary, not three; every id
    /// names its section and its question.
    #[test]
    fn every_check_id_round_trips_and_names_its_section() {
        for id in CheckId::ALL {
            let json = serde_json::to_string(&id).unwrap();
            assert_eq!(json, format!("\"{}\"", id.as_str()));
            assert_eq!(serde_json::from_str::<CheckId>(&json).unwrap(), id);
            assert_eq!(CheckId::parse(id.as_str()), Some(id));
            assert!(!id.section().is_empty());
            assert!(id.question().ends_with('?'), "{id}");
        }
        assert_eq!(CheckId::parse("split-brian"), None);
    }

    /// The severity vocabulary is the stable lowercase one, and its ladder
    /// is what a floor reads.
    #[test]
    fn severities_serialize_lowercase_and_climb_the_ladder() {
        for (s, w) in [
            (DoctorSeverity::Error, "error"),
            (DoctorSeverity::Warning, "warning"),
            (DoctorSeverity::Info, "info"),
        ] {
            assert_eq!(serde_json::to_value(s).unwrap(), w);
            assert_eq!(s.as_str(), w);
        }
        assert!(DoctorSeverity::Error.reaches(DoctorSeverity::Warning));
        assert!(!DoctorSeverity::Info.reaches(DoctorSeverity::Warning));
    }
}

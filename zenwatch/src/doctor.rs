//! The scheduled doctor (#390): the deployment's conformance checks asked
//! every few hours, and only what *changed* told to anyone.
//!
//! `run_doctor`'s stable check ids — zk2's since the port (#614, FL2):
//! `split-brain`, `binding-unsatisfied`, `contract-drift`, `token-missing`,
//! the `health-*` four and the rest of [`CheckId::ALL`] — are properties of
//! a deployment, and the failures that matter are on the fleet that has run
//! for three weeks: a provider upgraded on four hosts and not the fifth
//! (`contract-drift`), a role whose last provider went away
//! (`binding-unsatisfied`), an instance whose tokens disagree with its
//! descriptor (`token-missing`). This module runs the same doctor on an
//! interval and turns each run into notices for the engine's one `Notice`
//! stream — the same [`route`], discipline and sinks as everything else.
//!
//! [`route`]: crate::engine::route
//!
//! **A finding true since deployment is not news.** The first run is the
//! **baseline**: one `info` notification counting what it found, and every
//! finding entered into the discipline's ledger *assumed* — announced
//! without a delivery ([`crate::discipline::NoticeMeta::assume`]) — so that
//! the run which fixes one can say `resolved`. Every later run is judged
//! against the findings standing from the runs before it: one notification
//! per **new** finding, one `resolved` per **fixed** one, and nothing at
//! all when the two sets are empty. Findings are identified by `(check,
//! subject)`, which is the notice identity — evidence drift is the same
//! finding.
//!
//! **A fix is a fix only where the check was established.** A finding
//! absent from a run whose check could not be established — unobservable,
//! or not asked — has not been fixed: nobody saw it go. It stays standing,
//! carried to the next run's comparison, and is never announced
//! `resolved` (the reading `zenkey_fleet::doctor_delta`'s doc asks of a
//! notifier).
//!
//! **The four poles ride through** (RFC 13 §1). A run that judged nothing
//! because nothing was in scope (#510, the report's `unobservable`) is an
//! `unobservable` notification naming why, and the previous report is
//! **retained**, never replaced with nothing; the run after it is
//! `observable_again`. Inside a report, what was *not asked* is stated,
//! never read as clean: the coverage line every notification ends with
//! names the scope the run read and how many checks were established,
//! unobservable or not asked, and the published report carries each
//! check's verdict exactly as `zenctl doctor --format json` prints it.
//!
//! **Published.** After every run, [`ZenwatchDoctor`] goes out on
//! `zenwatch.v1`'s `state/doctor` (#614, FL3: no horizon, never re-put, not
//! deleted at a clean stop), so the last report is inspectable without SSH
//! and a `next_at` in the past is a schedule that lapsed. **Hours, not
//! seconds**: a run is a fan-in sweep, and it runs beside the drain, never
//! on it.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use zenkey_fleet::report::{CheckId, DoctorFinding, DoctorReport, DoctorSeverity, Judgement};
use zenkey_fleet::{CondState, DoctorDelta, DoctorSpec};

use crate::config::DoctorConfig;
use crate::discipline::severity_rank;
use crate::publish::{DoctorOutcome, DoctorStatus, ZenwatchDoctor};
use crate::render::{RenderConfig, RenderSource, state_word, truncate_bytes};
use crate::rules::{DOCTOR_RULE, Rule};
use crate::sinks::{NoticeKind, Notification, Outgoing};

/// The identity of the run itself — `unobservable` when it could not
/// happen, `observable_again` when it could once more.
pub const RUN_ID: &str = "doctor:run";
/// The baseline notification's id: no identity to remember, delivered once.
pub const BASELINE_ID: &str = "doctor:baseline";

/// One thing a doctor run said that the engine turns into an [`Outgoing`]
/// under the scheduled rule.
#[derive(Debug, Clone, PartialEq)]
pub enum DoctorNotice {
    /// The first successful run: what the deployment looks like now. One
    /// notification, `info`, counting findings — never one per finding.
    Baseline {
        at: String,
        findings: usize,
        checks: usize,
        /// `- <severity> <check> <subject>: <evidence>`, one per finding.
        lines: Vec<String>,
        coverage: String,
    },
    /// One finding, in the state this run put it in: `Firing` for a new
    /// one (or an assumed baseline one), `Ok` for a fixed one.
    Finding {
        finding: DoctorFinding,
        state: CondState,
        prior: Option<CondState>,
        /// Baseline: remembered as firing, delivered to nobody.
        assume: bool,
        /// The previous run, when there was one — what "since" means.
        since_run: Option<String>,
        coverage: String,
    },
    /// The run itself: `Err` is a run that could not happen (the report
    /// before it retained), `Ok` the run after such a failure.
    Run {
        at: String,
        outcome: Result<String, String>,
    },
}

impl DoctorNotice {
    /// The baseline has no identity: delivered once, never deduplicated.
    pub fn is_passthrough(&self) -> bool {
        matches!(self, DoctorNotice::Baseline { .. })
    }

    /// A baseline finding: remembered, not told.
    pub fn is_assumed(&self) -> bool {
        matches!(self, DoctorNotice::Finding { assume: true, .. })
    }
}

/// The wire word for a finding's severity — the config's vocabulary.
pub fn severity_word(s: DoctorSeverity) -> &'static str {
    s.as_str()
}

/// The notice identity of a finding: `doctor:<check>:<subject>` — the
/// `(check, subject)` key the delta uses, under the rule's id.
pub fn finding_id(check: CheckId, subject: &str) -> String {
    format!("{DOCTOR_RULE}:{check}:{subject}")
}

/// The inverse of [`finding_id`]: the check and subject an id names, for
/// an entry the state file remembered and this run did not find.
pub fn id_parts(id: &str) -> Option<(CheckId, &str)> {
    let rest = id.strip_prefix(DOCTOR_RULE)?.strip_prefix(':')?;
    let (check, subject) = rest.split_once(':')?;
    Some((CheckId::parse(check)?, subject))
}

/// Whether a run established `check` — a finding, or a clean answer — so
/// that a finding of it absent from the run is fixed, not merely unseen.
pub fn established(report: &DoctorReport, check: CheckId) -> bool {
    report.check(check).is_some_and(|c| {
        matches!(
            c.verdict,
            Judgement::Established | Judgement::NotEstablished { .. }
        )
    })
}

/// What the run covered, with every not-asked pole said out loud (RFC 13
/// §1, §3 O4): the presence read and whether it completed, the routers'
/// admin space, the `health.v1` reading and its clock, and how many checks
/// were established, unobservable or not asked — never a silent zero.
pub fn coverage(r: &DoctorReport) -> String {
    let presence = match r.scope.presence.as_option() {
        None => "not asked".to_string(),
        Some(p) => format!(
            "{} ({}), {} service(s), {} instance(s), {} token(s), {} undescribed, \
             {} revision(s) ({} held)",
            p.selector,
            if p.complete {
                "complete"
            } else {
                "possibly incomplete"
            },
            p.services,
            p.instances,
            p.tokens,
            p.undescribed,
            p.revisions,
            p.held
        ),
    };
    let routers = match r.scope.routers.as_option() {
        None => "not asked".to_string(),
        Some(n) => format!("{n} answered"),
    };
    let health = match r.scope.health.as_option() {
        None => "not asked".to_string(),
        Some(h) => format!(
            "{} service(s), clocks {}",
            h.services,
            if h.clocks_synced {
                "synced (the operator's word)"
            } else {
                "not vouched for"
            }
        ),
    };
    let config = match r.scope.config.as_option() {
        None => "not asked".to_string(),
        Some(c) => format!(
            "{} service(s), {} reading(s), clocks {}",
            c.services,
            c.readings,
            if c.clocks_synced {
                "synced (the operator's word)"
            } else {
                "not vouched for"
            }
        ),
    };
    let (mut est, mut unobs, mut not_asked) = (0, Vec::new(), 0);
    for c in &r.checks {
        match &c.verdict {
            Judgement::Established | Judgement::NotEstablished { .. } => est += 1,
            Judgement::Unobservable { .. } => unobs.push(c.check.as_str()),
            Judgement::NotAsked => not_asked += 1,
        }
    }
    let unobservable = if unobs.is_empty() {
        "0 unobservable".to_string()
    } else {
        format!("{} unobservable ({})", unobs.len(), unobs.join(", "))
    };
    let namespace = if r.scope.namespace.is_empty() {
        "the bus root".to_string()
    } else {
        format!("namespace {:?}", r.scope.namespace)
    };
    format!(
        "coverage: {namespace}; presence: {presence}; routers: {routers}; health: {health}; \
         config: {config}; checks: {est} established, {unobservable}, {not_asked} not asked"
    )
}

fn rfc3339(now: f64) -> String {
    zenkey_fleet::rfc3339_from_unix(now.max(0.0) as u64)
}

/// The last successful run, kept whole: what a failed one retains, and the
/// findings standing after it — its own, plus those of checks it could not
/// establish, carried from the runs before.
#[derive(Debug, Clone)]
struct Previous {
    report: DoctorReport,
    standing: Vec<DoctorFinding>,
    at: String,
}

/// The schedule's state between runs: the previous report, whether the
/// last run failed, and the document last published.
#[derive(Debug)]
pub struct Schedule {
    cfg: DoctorConfig,
    every: Duration,
    timeout: Duration,
    clocks_synced: bool,
    prev: Option<Previous>,
    /// The error of the last run, while it stands.
    failed: Option<String>,
    runs: u64,
    last: Option<ZenwatchDoctor>,
}

impl Schedule {
    /// From a checked config: `every()` is `Some` because [`crate::config::check`]
    /// refused anything else. `timeout` is the bus's, unless the block says;
    /// `clocks_synced` is the config's top-level word (`freshness.v1` §2.6).
    pub fn new(cfg: &DoctorConfig, bus_timeout: Duration, clocks_synced: bool) -> Schedule {
        Schedule {
            every: cfg.every().unwrap_or(Duration::from_secs(3600)),
            timeout: cfg
                .timeout_s
                .map(Duration::from_secs_f64)
                .unwrap_or(bus_timeout),
            clocks_synced,
            cfg: cfg.clone(),
            prev: None,
            failed: None,
            runs: 0,
            last: None,
        }
    }

    pub fn every(&self) -> Duration {
        self.every
    }

    /// What one run should ask: every check, `deep` as the block says
    /// (the data-plane checks, `state-stamp-foreign` and
    /// `population-over-bound`), `clocks_synced` as the config says, and
    /// the timeout; every other setting the doctor's default.
    pub fn spec(&self) -> DoctorSpec {
        let mut spec = DoctorSpec::new(self.timeout);
        spec.deep = self.cfg.deep;
        spec.clocks_synced = self.clocks_synced;
        spec
    }

    pub fn runs(&self) -> u64 {
        self.runs
    }

    /// The document last published, if a run has happened.
    pub fn document(&self) -> Option<&ZenwatchDoctor> {
        self.last.as_ref()
    }

    /// For the `health.v1` check: not yet run, the last run ok, or failed.
    pub fn status(&self) -> DoctorStatus {
        match (&self.last, &self.failed) {
            (None, _) => DoctorStatus::Pending,
            (Some(_), Some(_)) => DoctorStatus::Failed,
            (Some(_), None) => DoctorStatus::Ok,
        }
    }

    /// Whether a finding clears the block's severity floor.
    fn eligible(&self, f: &DoctorFinding) -> bool {
        severity_rank(severity_word(f.severity)) >= self.cfg.floor_rank()
    }

    /// One run's outcome in, the notices out, and the document updated.
    ///
    /// `announced` is what the discipline currently has announced under the
    /// doctor rule — after a restart, the state file's memory — so that an
    /// entry remembered firing which this run establishes gone is resolved
    /// rather than left announced forever, and a remembered failed run is
    /// `observable_again` on the first success.
    pub fn observe(
        &mut self,
        outcome: Result<DoctorReport, String>,
        now: f64,
        announced: &[String],
    ) -> Vec<DoctorNotice> {
        self.runs += 1;
        // A run that judged nothing (#510) is one that could not see: the
        // same `unobservable` notice as a failed one, the previous report
        // retained — never an empty baseline every finding is fixed against.
        let outcome = outcome.and_then(|r| match r.unobservable.clone() {
            Some(why) => Err(why),
            None => Ok(r),
        });
        let ran_at = rfc3339(now);
        let next_at = rfc3339(now + self.every.as_secs_f64());
        let every_s = self.every.as_secs_f64();
        let mut out = Vec::new();
        let report = match outcome {
            Err(error) => {
                out.push(DoctorNotice::Run {
                    at: ran_at.clone(),
                    outcome: Err(error.clone()),
                });
                self.failed = Some(error.clone());
                self.last = Some(ZenwatchDoctor {
                    ran_at,
                    next_at,
                    every_s,
                    outcome: DoctorOutcome::Failed,
                    error: Some(error),
                    findings: self
                        .prev
                        .as_ref()
                        .map_or(0, |p| p.report.findings().count()),
                    new: 0,
                    fixed: 0,
                    report: self
                        .prev
                        .as_ref()
                        .map(|p| serde_json::to_value(&p.report).expect("a report serializes"))
                        .unwrap_or(serde_json::Value::Null),
                    report_at: self.prev.as_ref().map(|p| p.at.clone()),
                    delta: None,
                });
                return out;
            }
            Ok(report) => report,
        };
        if self.failed.take().is_some() || announced.iter().any(|id| id == RUN_ID) {
            out.push(DoctorNotice::Run {
                at: ran_at.clone(),
                outcome: Ok(coverage(&report)),
            });
        }
        let cov = coverage(&report);
        let found: Vec<DoctorFinding> = report.findings().cloned().collect();
        let key = |f: &DoctorFinding| (f.check, f.subject.clone());
        let (new, fixed, delta, standing) = match &self.prev {
            None => {
                let checks = found.iter().map(|f| f.check).collect::<BTreeSet<_>>().len();
                out.push(DoctorNotice::Baseline {
                    at: ran_at.clone(),
                    findings: found.len(),
                    checks,
                    lines: found
                        .iter()
                        .map(|f| {
                            format!(
                                "- {} {} {}: {}",
                                severity_word(f.severity),
                                f.check,
                                f.subject,
                                f.evidence
                            )
                        })
                        .collect(),
                    coverage: cov.clone(),
                });
                for f in found.iter().filter(|f| self.eligible(f)) {
                    out.push(DoctorNotice::Finding {
                        finding: f.clone(),
                        state: CondState::Firing,
                        prior: None,
                        assume: true,
                        since_run: None,
                        coverage: cov.clone(),
                    });
                }
                // Remembered firing before a restart, absent now: fixed
                // while nobody watched — resolved, not announced forever —
                // when this run established its check; carried otherwise.
                let mut carried = Vec::new();
                for id in announced {
                    let Some((check, subject)) = id_parts(id) else {
                        continue;
                    };
                    if found
                        .iter()
                        .any(|f| f.check == check && f.subject == subject)
                    {
                        continue;
                    }
                    let remembered = DoctorFinding {
                        severity: DoctorSeverity::Info,
                        check,
                        subject: subject.to_string(),
                        evidence: "remembered firing by the state file; absent from the \
                                   first run after the restart"
                            .into(),
                    };
                    if !established(&report, check) {
                        carried.push(remembered);
                        continue;
                    }
                    out.push(DoctorNotice::Finding {
                        finding: remembered,
                        state: CondState::Ok,
                        prior: Some(CondState::Firing),
                        assume: false,
                        since_run: None,
                        coverage: cov.clone(),
                    });
                }
                let mut standing = found.clone();
                standing.extend(carried);
                (0, 0, None, standing)
            }
            Some(prev) => {
                let before: BTreeSet<(CheckId, String)> = prev.standing.iter().map(key).collect();
                let now_found: BTreeSet<(CheckId, String)> = found.iter().map(key).collect();
                let new: Vec<DoctorFinding> = found
                    .iter()
                    .filter(|f| !before.contains(&key(f)))
                    .cloned()
                    .collect();
                let (fixed, carried): (Vec<DoctorFinding>, Vec<DoctorFinding>) = prev
                    .standing
                    .iter()
                    .filter(|f| !now_found.contains(&key(f)))
                    .cloned()
                    .partition(|f| established(&report, f.check));
                for f in new.iter().filter(|f| self.eligible(f)) {
                    out.push(DoctorNotice::Finding {
                        finding: f.clone(),
                        state: CondState::Firing,
                        prior: Some(CondState::Ok),
                        assume: false,
                        since_run: Some(prev.at.clone()),
                        coverage: cov.clone(),
                    });
                }
                for f in fixed.iter().filter(|f| self.eligible(f)) {
                    out.push(DoctorNotice::Finding {
                        finding: f.clone(),
                        state: CondState::Ok,
                        prior: Some(CondState::Firing),
                        assume: false,
                        since_run: Some(prev.at.clone()),
                        coverage: cov.clone(),
                    });
                }
                let unchanged = now_found.intersection(&before).count();
                let delta = DoctorDelta {
                    new: new.clone(),
                    fixed: fixed.clone(),
                    unchanged,
                };
                let mut standing = found.clone();
                standing.extend(carried);
                (
                    new.len(),
                    fixed.len(),
                    Some(serde_json::to_value(&delta).expect("a delta serializes")),
                    standing,
                )
            }
        };
        self.last = Some(ZenwatchDoctor {
            ran_at: ran_at.clone(),
            next_at,
            every_s,
            outcome: DoctorOutcome::Ok,
            error: None,
            findings: found.len(),
            new,
            fixed,
            report: serde_json::to_value(&report).expect("a report serializes"),
            report_at: Some(ran_at.clone()),
            delta,
        });
        self.prev = Some(Previous {
            report,
            standing,
            at: ran_at,
        });
        out
    }
}

/// One doctor notice as the [`Outgoing`] the scheduled rule routes —
/// rendered here rather than in [`crate::render`] because a doctor finding
/// is a judgement over a sweep, not a payload on a key: the message is the
/// check and the section it enforces, the subject, the evidence and the
/// coverage, bounded by the same byte limit.
pub fn outgoing(notice: &DoctorNotice, rule: &Rule, render: &RenderConfig) -> Outgoing {
    let at = zenkey_fleet::rfc3339_now();
    let (id, kind, state, prior, severity, title, lines, labels, evidence) = match notice {
        DoctorNotice::Baseline {
            at: ran_at,
            findings,
            checks,
            lines,
            coverage,
        } => {
            let state = if *findings > 0 {
                CondState::Firing
            } else {
                CondState::Ok
            };
            let summary =
                format!("doctor baseline: {findings} finding(s) across {checks} check(s)");
            let mut body = vec![
                format!("state: {} (first observation)", state_word(state)),
                format!(
                    "{summary} at {ran_at} — what the deployment looks like now; findings true \
                     since deployment are not news, and only what changes from here is"
                ),
            ];
            body.extend(lines.iter().cloned());
            body.push(coverage.clone());
            (
                BASELINE_ID.to_string(),
                NoticeKind::Doctor,
                state,
                None,
                "info".to_string(),
                summary.clone(),
                body,
                BTreeMap::new(),
                summary,
            )
        }
        DoctorNotice::Finding {
            finding,
            state,
            prior,
            assume,
            since_run,
            coverage,
        } => {
            let since = match (state, since_run, assume) {
                (_, _, true) => "baseline".to_string(),
                (CondState::Ok, Some(run), _) => format!("gone since the run at {run}"),
                (CondState::Ok, None, _) => "gone".to_string(),
                (_, Some(run), _) => format!("new since the run at {run}"),
                (_, None, _) => "first observation".to_string(),
            };
            let head = match prior {
                Some(p) => format!(
                    "state: {} (was: {}; {since})",
                    state_word(*state),
                    state_word(*p)
                ),
                None => format!("state: {} ({since})", state_word(*state)),
            };
            let severity = severity_word(finding.severity).to_string();
            (
                finding_id(finding.check, &finding.subject),
                NoticeKind::Doctor,
                *state,
                *prior,
                severity.clone(),
                format!("{} {}", finding.check, finding.subject),
                vec![
                    head,
                    format!("check: {} ({})", finding.check, finding.check.section()),
                    format!("question: {}", finding.check.question()),
                    format!("subject: {}", finding.subject),
                    format!("evidence: {}", finding.evidence),
                    coverage.clone(),
                ],
                BTreeMap::from([("check".to_string(), finding.check.to_string())]),
                format!(
                    "{} {}: {}",
                    finding.check, finding.subject, finding.evidence
                ),
            )
        }
        DoctorNotice::Run {
            at: ran_at,
            outcome,
        } => match outcome {
            Err(error) => (
                RUN_ID.to_string(),
                NoticeKind::Unobservable,
                CondState::Unobservable,
                Some(CondState::Ok),
                "warning".to_string(),
                "the doctor could not run".to_string(),
                vec![
                    "state: unobservable (was: ok)".to_string(),
                    format!("the doctor could not run at {ran_at}: {error}"),
                    "the previous report is retained on zenwatch.v1's state/doctor; nothing \
                     here says the deployment is clean (RFC 13 §1)"
                        .to_string(),
                ],
                BTreeMap::new(),
                format!("the doctor could not run: {error}"),
            ),
            Ok(coverage) => (
                RUN_ID.to_string(),
                NoticeKind::ObservableAgain,
                CondState::Ok,
                Some(CondState::Unobservable),
                "warning".to_string(),
                "the doctor ran again".to_string(),
                vec![
                    "state: ok (was: unobservable)".to_string(),
                    format!(
                        "the doctor ran at {ran_at}; its findings are judged against the \
                             last report that succeeded"
                    ),
                    coverage.clone(),
                ],
                BTreeMap::new(),
                "the doctor ran again".to_string(),
            ),
        },
    };
    let (message, truncated) = truncate_bytes(&lines.join("\n"), render.max_message_bytes);
    Outgoing {
        notification: Notification {
            id,
            rule: rule.name.clone(),
            rule_kind: rule.kind.head().to_string(),
            kind,
            state,
            prior,
            severity,
            title,
            message,
            labels,
            at,
            evidence,
            rendering: RenderSource::KeyOnly,
            truncated,
            repeat: 0,
            group: None,
            presence: None,
            alert: None,
        },
        sinks: rule.sinks.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DisciplineConfig;
    use crate::discipline::{Discipline, NoticeMeta};
    use zenkey_fleet::report::{
        Asked, CheckReport, DoctorHealth, DoctorPresence, DoctorScope, Unjudged,
    };

    fn finding(check: CheckId, subject: &str, severity: DoctorSeverity) -> DoctorFinding {
        DoctorFinding {
            severity,
            check,
            subject: subject.into(),
            evidence: format!("evidence for {subject}"),
        }
    }

    fn scope(presence: bool) -> DoctorScope {
        DoctorScope {
            namespace: "prod".into(),
            presence: if presence {
                Asked::Asked(DoctorPresence {
                    selector: "zk2/*/*/@zk/**".into(),
                    complete: true,
                    grace_s: 2.0,
                    services: 3,
                    instances: 3,
                    tokens: 7,
                    undescribed: 0,
                    revisions: 2,
                    held: 2,
                })
            } else {
                Asked::NotAsked
            },
            routers: Asked::NotAsked,
            health: Asked::NotAsked,
            config: Asked::NotAsked,
        }
    }

    /// A zk2 report: every check asked and clean but for `findings`, each
    /// under its own check, and `unseen` unobservable.
    fn report_with(findings: Vec<DoctorFinding>, unseen: &[CheckId]) -> DoctorReport {
        DoctorReport {
            scope: scope(true),
            checks: CheckId::ALL
                .into_iter()
                .map(|id| {
                    if unseen.contains(&id) {
                        return CheckReport::of(
                            id,
                            vec![],
                            vec![Unjudged {
                                subject: "fleet".into(),
                                reason: "a read ended at its timeout".into(),
                            }],
                            "unused",
                        );
                    }
                    let mine = findings.iter().filter(|f| f.check == id).cloned().collect();
                    CheckReport::of(id, mine, vec![], "clean")
                })
                .collect(),
            unobservable: None,
        }
    }

    fn report(findings: Vec<DoctorFinding>) -> DoctorReport {
        report_with(findings, &[])
    }

    fn cfg(floor: Option<&str>) -> DoctorConfig {
        DoctorConfig {
            every_h: Some(6.0),
            every_s: None,
            deep: false,
            sample: None,
            timeout_s: None,
            sinks: vec!["ops".into()],
            severity_floor: floor.map(str::to_string),
        }
    }

    /// The whole path a notice takes in the engine: schedule → outgoing →
    /// discipline → flush, with an injected clock.
    struct Harness {
        schedule: Schedule,
        rule: Rule,
        discipline: Discipline,
        render: RenderConfig,
    }

    impl Harness {
        fn new(floor: Option<&str>) -> Harness {
            let cfg = cfg(floor);
            let rule = Rule::scheduled_doctor(&cfg);
            let discipline = Discipline::new(
                &DisciplineConfig {
                    group_window_s: 0.0,
                    ..DisciplineConfig::default()
                },
                std::slice::from_ref(&rule),
                &RenderConfig::default(),
                100,
            );
            Harness {
                schedule: Schedule::new(&cfg, Duration::from_secs(2), false),
                rule,
                discipline,
                render: RenderConfig::default(),
            }
        }

        fn run(&mut self, outcome: Result<DoctorReport, String>, now: f64) -> Vec<Outgoing> {
            let announced = self.discipline.announced_under(DOCTOR_RULE);
            for n in self.schedule.observe(outcome, now, &announced) {
                let meta = NoticeMeta {
                    address: None,
                    passthrough: n.is_passthrough(),
                    assume: n.is_assumed(),
                };
                let o = outgoing(&n, &self.rule, &self.render);
                self.discipline.observe(o, &meta, now);
            }
            self.discipline.tick(now, false).outgoing
        }
    }

    fn ids(out: &[Outgoing]) -> Vec<(String, NoticeKind, CondState)> {
        out.iter()
            .map(|o| {
                (
                    o.notification.id.clone(),
                    o.notification.kind,
                    o.notification.state,
                )
            })
            .collect()
    }

    /// The spec a run asks: every check, `deep` and `clocks_synced` as the
    /// config says (#614, FL2).
    #[test]
    fn the_spec_asks_every_check_with_the_config_s_deep_and_clock() {
        let mut c = cfg(None);
        let s = Schedule::new(&c, Duration::from_secs(2), false).spec();
        assert!(!s.deep && !s.clocks_synced);
        assert_eq!(s.checks.len(), CheckId::ALL.len());
        assert!(!s.asks(CheckId::StateStampForeign), "not deep");
        c.deep = true;
        let s = Schedule::new(&c, Duration::from_secs(2), true).spec();
        assert!(s.deep && s.clocks_synced);
        assert!(s.asks(CheckId::StateStampForeign) && s.asks(CheckId::PopulationOverBound));
        assert_eq!(s.timeout, Duration::from_secs(2));
    }

    /// Baseline, then deltas: the first run is one `info` notification
    /// counting the findings and naming them, not one per finding; an
    /// identical second run is nothing; a third with one new and one fixed
    /// is one `doctor` firing and one `resolved`, each naming its check and
    /// subject; and the published document counts each step.
    #[test]
    fn a_baseline_then_only_deltas() {
        let mut h = Harness::new(None);
        let a = finding(
            CheckId::BindingUnsatisfied,
            "ws-01/gui scenario",
            DoctorSeverity::Error,
        );
        let b = finding(
            CheckId::ContractUnavailable,
            "tc.v1 sha256:00",
            DoctorSeverity::Info,
        );
        let out = h.run(Ok(report(vec![a.clone(), b.clone()])), 0.0);
        assert_eq!(out.len(), 1, "one baseline, not one per finding: {out:?}");
        let n = &out[0].notification;
        assert_eq!(n.id, BASELINE_ID);
        assert_eq!(n.kind, NoticeKind::Doctor);
        assert_eq!(n.severity, "info");
        assert_eq!(n.state, CondState::Firing);
        assert_eq!(n.prior, None);
        assert_eq!(n.rule, "doctor");
        assert_eq!(n.rule_kind, "doctor");
        assert_eq!(n.title, "doctor baseline: 2 finding(s) across 2 check(s)");
        assert!(
            n.message.contains(
                "- error binding-unsatisfied ws-01/gui scenario: evidence for ws-01/gui scenario"
            ),
            "{}",
            n.message
        );
        assert!(
            n.message
                .contains("checks: 27 established, 0 unobservable, 0 not asked"),
            "{}",
            n.message
        );
        assert_eq!(out[0].sinks, vec!["ops".to_string()]);
        // Both findings are remembered firing under the rule, delivered to nobody.
        assert_eq!(
            h.discipline.announced_under(DOCTOR_RULE),
            vec![
                "doctor:binding-unsatisfied:ws-01/gui scenario".to_string(),
                "doctor:contract-unavailable:tc.v1 sha256:00".to_string(),
            ]
        );
        let doc = h.schedule.document().unwrap().clone();
        assert_eq!(doc.outcome, DoctorOutcome::Ok);
        assert_eq!((doc.findings, doc.new, doc.fixed), (2, 0, 0));
        assert_eq!(doc.delta, None);
        assert_eq!(doc.every_s, 6.0 * 3600.0);
        assert_eq!(doc.report["scope"]["namespace"], "prod");
        assert_eq!(doc.report["checks"][1]["check"], "binding-unsatisfied");
        assert_eq!(doc.report["checks"][1]["verdict"]["answer"], "established");
        assert_eq!(doc.report_at.as_deref(), Some(doc.ran_at.as_str()));

        // The same again: nothing.
        let out = h.run(Ok(report(vec![a.clone(), b.clone()])), 100.0);
        assert!(out.is_empty(), "{out:?}");
        let doc = h.schedule.document().unwrap();
        assert_eq!((doc.findings, doc.new, doc.fixed), (2, 0, 0));
        assert_eq!(doc.delta.as_ref().unwrap()["unchanged"], 2);

        // A drifts to a different wording (same finding), B is fixed, C is new.
        let mut a2 = a.clone();
        a2.evidence = "different wording".into();
        let c = finding(
            CheckId::TokenMissing,
            "host-a/tc@0000000000000001 tc.netif.v1",
            DoctorSeverity::Error,
        );
        let out = h.run(Ok(report(vec![a2, c])), 200.0);
        assert_eq!(
            ids(&out),
            vec![
                (
                    "doctor:contract-unavailable:tc.v1 sha256:00".to_string(),
                    NoticeKind::Resolved,
                    CondState::Ok
                ),
                (
                    "doctor:token-missing:host-a/tc@0000000000000001 tc.netif.v1".to_string(),
                    NoticeKind::Doctor,
                    CondState::Firing
                ),
            ]
        );
        let new = &out[1].notification;
        assert_eq!(
            new.title,
            "token-missing host-a/tc@0000000000000001 tc.netif.v1"
        );
        assert_eq!(new.severity, "error");
        assert_eq!(new.prior, Some(CondState::Ok));
        assert_eq!(
            new.labels.get("check").map(String::as_str),
            Some("token-missing")
        );
        assert!(
            new.message.contains("check: token-missing (§8.1)"),
            "{}",
            new.message
        );
        assert!(
            new.message.contains("new since the run at "),
            "{}",
            new.message
        );
        assert!(
            new.message
                .contains("evidence: evidence for host-a/tc@0000000000000001 tc.netif.v1")
        );
        let fixed = &out[0].notification;
        assert_eq!(fixed.title, "contract-unavailable tc.v1 sha256:00");
        assert_eq!(fixed.prior, Some(CondState::Firing));
        let doc = h.schedule.document().unwrap();
        assert_eq!((doc.findings, doc.new, doc.fixed), (2, 1, 1));
        assert_eq!(
            doc.delta.as_ref().unwrap()["new"][0]["check"],
            "token-missing"
        );
        assert_eq!(
            doc.delta.as_ref().unwrap()["fixed"][0]["check"],
            "contract-unavailable"
        );
        assert_eq!(h.schedule.runs(), 3);
    }

    /// A fix is a fix only where the check was established: a finding
    /// absent from a run whose check was unobservable is not `resolved` —
    /// nobody saw it go — and stays standing, so it is neither new when it
    /// shows again nor lost when the check is established clean, which is
    /// the run that resolves it.
    #[test]
    fn an_unobservable_check_is_never_a_fix() {
        let mut h = Harness::new(None);
        let a = finding(CheckId::SplitBrain, "h1/tc tc.v1", DoctorSeverity::Error);
        h.run(Ok(report(vec![a.clone()])), 0.0);
        let out = h.run(Ok(report_with(vec![], &[CheckId::SplitBrain])), 100.0);
        assert!(out.is_empty(), "unseen, not fixed: {out:?}");
        let doc = h.schedule.document().unwrap();
        assert_eq!((doc.new, doc.fixed), (0, 0));
        assert!(
            h.discipline
                .announced_under(DOCTOR_RULE)
                .contains(&"doctor:split-brain:h1/tc tc.v1".to_string()),
            "still remembered firing"
        );
        // Seen again: not new.
        assert!(h.run(Ok(report(vec![a.clone()])), 200.0).is_empty());
        // Unseen again, then established clean: the clean run resolves it.
        assert!(
            h.run(Ok(report_with(vec![], &[CheckId::SplitBrain])), 300.0)
                .is_empty()
        );
        let out = h.run(Ok(report(vec![])), 400.0);
        assert_eq!(
            ids(&out),
            vec![(
                "doctor:split-brain:h1/tc tc.v1".to_string(),
                NoticeKind::Resolved,
                CondState::Ok
            )]
        );
        assert!(
            out[0]
                .notification
                .message
                .contains("checks: 27 established"),
            "{}",
            out[0].notification.message
        );
    }

    /// A run that fails is `unobservable`, naming the error, with the
    /// previous report retained in the document (and its findings still
    /// counted); the run after it is `observable_again`, and its findings
    /// are judged against the last report that succeeded — not against
    /// nothing.
    #[test]
    fn a_failed_run_is_unobservable_and_keeps_the_baseline() {
        let mut h = Harness::new(None);
        let a = finding(CheckId::SplitBrain, "h1/tc tc.v1", DoctorSeverity::Error);
        h.run(Ok(report(vec![a.clone()])), 0.0);
        assert_eq!(h.schedule.status(), DoctorStatus::Ok);

        let out = h.run(Err("no session in the namespace".into()), 100.0);
        assert_eq!(
            ids(&out),
            vec![(
                RUN_ID.to_string(),
                NoticeKind::Unobservable,
                CondState::Unobservable
            )]
        );
        let n = &out[0].notification;
        assert_eq!(n.title, "the doctor could not run");
        assert!(
            n.message.contains("no session in the namespace"),
            "{}",
            n.message
        );
        assert!(n.message.contains("retained"), "{}", n.message);
        assert_eq!(h.schedule.status(), DoctorStatus::Failed);
        let doc = h.schedule.document().unwrap().clone();
        assert_eq!(doc.outcome, DoctorOutcome::Failed);
        assert_eq!(doc.error.as_deref(), Some("no session in the namespace"));
        assert_eq!(doc.findings, 1, "the baseline's findings, retained");
        assert_eq!(
            doc.report["checks"][0]["findings"][0]["subject"],
            "h1/tc tc.v1"
        );
        assert_ne!(doc.report_at, Some(doc.ran_at.clone()), "an older report");
        assert_eq!(doc.delta, None);
        // Still remembered firing.
        assert_eq!(h.discipline.announced_under(DOCTOR_RULE).len(), 2);

        // A second failure is a duplicate, not a second page.
        assert!(h.run(Err("still down".into()), 150.0).is_empty());

        // Recovery: observable again, and the baseline finding is now
        // fixed — judged against the run at 0, not against nothing.
        let out = h.run(Ok(report(vec![])), 200.0);
        assert_eq!(
            ids(&out),
            vec![
                (
                    RUN_ID.to_string(),
                    NoticeKind::ObservableAgain,
                    CondState::Ok
                ),
                (
                    "doctor:split-brain:h1/tc tc.v1".to_string(),
                    NoticeKind::Resolved,
                    CondState::Ok
                ),
            ]
        );
        assert_eq!(h.schedule.status(), DoctorStatus::Ok);
        let doc = h.schedule.document().unwrap();
        assert_eq!(doc.outcome, DoctorOutcome::Ok);
        assert_eq!((doc.findings, doc.new, doc.fixed), (0, 0, 1));

        // A run that fails first, with no baseline yet: unobservable, an
        // empty document, and the first success is the baseline.
        let mut h = Harness::new(None);
        let out = h.run(Err("no session".into()), 0.0);
        assert_eq!(out.len(), 1);
        let doc = h.schedule.document().unwrap().clone();
        assert_eq!(doc.report, serde_json::Value::Null);
        assert_eq!(doc.report_at, None);
        let out = h.run(Ok(report(vec![])), 100.0);
        assert_eq!(
            ids(&out).iter().map(|(_, k, _)| *k).collect::<Vec<_>>(),
            vec![NoticeKind::Doctor, NoticeKind::ObservableAgain],
            "flushed in id order: the baseline, then the run"
        );
        assert_eq!(out[0].notification.state, CondState::Ok, "a clean baseline");
    }

    /// #510: a run over an empty scope judged nothing — it reads exactly as
    /// a failed run does: `unobservable` with the reason, the baseline's
    /// report retained, and no `resolved` for findings it could not see.
    #[test]
    fn an_empty_scope_is_unobservable_never_a_fix() {
        let mut h = Harness::new(None);
        let a = finding(CheckId::SplitBrain, "h1/tc tc.v1", DoctorSeverity::Error);
        h.run(Ok(report(vec![a])), 0.0);
        let empty = DoctorReport {
            unobservable: Some("no zk2 token visible to this reader".into()),
            ..report(vec![])
        };
        let out = h.run(Ok(empty), 100.0);
        assert_eq!(
            ids(&out),
            vec![(
                RUN_ID.to_string(),
                NoticeKind::Unobservable,
                CondState::Unobservable
            )]
        );
        assert!(
            out[0]
                .notification
                .message
                .contains("no zk2 token visible to this reader"),
            "{}",
            out[0].notification.message
        );
        let doc = h.schedule.document().unwrap();
        assert_eq!(doc.outcome, DoctorOutcome::Failed);
        assert_eq!(doc.findings, 1, "the baseline's findings, retained");
    }

    /// What was not asked is stated, never read as clean: the coverage line
    /// says "not asked" for every scope half the run did not read and
    /// counts the checks it did not ask; the published report keeps the
    /// absences as the report contract pins them.
    #[test]
    fn not_asked_rides_through_never_as_clean() {
        let bare = DoctorReport {
            scope: scope(false),
            checks: CheckId::ALL
                .into_iter()
                .map(|id| {
                    if id == CheckId::SplitBrain {
                        CheckReport::of(id, vec![], vec![], "clean")
                    } else {
                        CheckReport::not_asked(id)
                    }
                })
                .collect(),
            unobservable: None,
        };
        let line = coverage(&bare);
        assert!(line.contains("presence: not asked"), "{line}");
        assert!(line.contains("routers: not asked"), "{line}");
        assert!(line.contains("health: not asked"), "{line}");
        assert!(
            line.contains("checks: 1 established, 0 unobservable, 26 not asked"),
            "{line}"
        );
        assert!(line.contains("namespace \"prod\""), "{line}");
        let mut h = Harness::new(None);
        let out = h.run(Ok(bare), 0.0);
        assert!(
            out[0].notification.message.contains("presence: not asked"),
            "{}",
            out[0].notification.message
        );
        let doc = h.schedule.document().unwrap();
        assert!(
            doc.report["scope"].get("presence").is_none(),
            "not asked is absence on the wire: {}",
            doc.report
        );
        let read = DoctorReport {
            scope: DoctorScope {
                routers: Asked::Asked(2),
                health: Asked::Asked(DoctorHealth {
                    selector: "zk2/*/*/health.v1/state/**".into(),
                    services: 1,
                    clocks_synced: true,
                }),
                config: Asked::NotAsked,
                namespace: String::new(),
                ..scope(true)
            },
            ..report_with(vec![], &[CheckId::HealthStale])
        };
        let line = coverage(&read);
        assert!(line.contains("the bus root"), "{line}");
        assert!(line.contains("(complete)"), "{line}");
        assert!(line.contains("routers: 2 answered"), "{line}");
        assert!(line.contains("clocks synced"), "{line}");
        assert!(line.contains("1 unobservable (health-stale)"), "{line}");
    }

    /// The severity floor: an `info` finding below a `warning` floor is in
    /// the report and its counts but is never a notification — new or
    /// fixed; the baseline still counts it.
    #[test]
    fn the_severity_floor_keeps_low_findings_in_the_report_only() {
        let mut h = Harness::new(Some("warning"));
        let info = finding(
            CheckId::BindingUnsatisfied,
            "ws-01/gui scenario",
            DoctorSeverity::Info,
        );
        let out = h.run(Ok(report(vec![info.clone()])), 0.0);
        assert!(
            out[0]
                .notification
                .title
                .starts_with("doctor baseline: 1 finding(s)")
        );
        assert!(
            h.discipline.announced_under(DOCTOR_RULE).is_empty(),
            "not remembered"
        );
        let warn = finding(
            CheckId::HealthDegraded,
            "host-a/tc",
            DoctorSeverity::Warning,
        );
        let out = h.run(Ok(report(vec![warn.clone()])), 100.0);
        assert_eq!(out.len(), 1, "the warning, not the fixed info: {out:?}");
        assert_eq!(out[0].notification.id, "doctor:health-degraded:host-a/tc");
        let doc = h.schedule.document().unwrap();
        assert_eq!(
            (doc.findings, doc.new, doc.fixed),
            (1, 1, 1),
            "the document counts everything"
        );
    }

    /// After a restart the state file remembers what was announced: a
    /// finding still present is assumed silently, one gone (its check
    /// established) is resolved, one whose check this run could not
    /// establish stays remembered, and a remembered failed run is
    /// observable again.
    #[test]
    fn a_restart_reconciles_the_baseline_against_the_ledger() {
        let mut h = Harness::new(None);
        // Stand in for the restored ledger: three findings announced, and
        // the run itself unobservable.
        let present = finding(CheckId::SplitBrain, "h1/tc tc.v1", DoctorSeverity::Error);
        let gone = finding(CheckId::ContractDrift, "tc.netif.v1", DoctorSeverity::Error);
        let unseen = finding(CheckId::HealthStale, "host-b/tc", DoctorSeverity::Warning);
        for f in [&present, &gone, &unseen] {
            let o = outgoing(
                &DoctorNotice::Finding {
                    finding: f.clone(),
                    state: CondState::Firing,
                    prior: None,
                    assume: false,
                    since_run: None,
                    coverage: String::new(),
                },
                &h.rule,
                &h.render,
            );
            h.discipline.observe(o, &NoticeMeta::default(), -100.0);
        }
        let o = outgoing(
            &DoctorNotice::Run {
                at: "t".into(),
                outcome: Err("down".into()),
            },
            &h.rule,
            &h.render,
        );
        h.discipline.observe(o, &NoticeMeta::default(), -100.0);
        h.discipline.tick(-100.0, false);
        assert_eq!(h.discipline.announced_under(DOCTOR_RULE).len(), 4);

        let out = h.run(Ok(report_with(vec![present], &[CheckId::HealthStale])), 0.0);
        assert_eq!(
            ids(&out),
            vec![
                (
                    BASELINE_ID.to_string(),
                    NoticeKind::Doctor,
                    CondState::Firing
                ),
                (
                    "doctor:contract-drift:tc.netif.v1".to_string(),
                    NoticeKind::Resolved,
                    CondState::Ok
                ),
                (
                    RUN_ID.to_string(),
                    NoticeKind::ObservableAgain,
                    CondState::Ok
                ),
            ]
        );
        assert_eq!(
            h.discipline.announced_under(DOCTOR_RULE),
            vec![
                "doctor:health-stale:host-b/tc".to_string(),
                "doctor:split-brain:h1/tc tc.v1".to_string(),
            ],
            "the unseen one stays remembered"
        );
        assert_eq!(
            id_parts("doctor:split-brain:h1/tc tc.v1:with:colons"),
            Some((CheckId::SplitBrain, "h1/tc tc.v1:with:colons"))
        );
        assert_eq!(id_parts("doctor:run"), None);
        assert_eq!(id_parts("doctor:slice-sync:h-1/sysinfo"), None, "v1's ids");
        assert_eq!(id_parts("tc-gone:host-a/tc"), None);
    }

    /// The message is bounded like every rendering: a baseline with many
    /// findings is cut at the byte limit and says so.
    #[test]
    fn the_baseline_is_bounded() {
        let cfg = cfg(None);
        let rule = Rule::scheduled_doctor(&cfg);
        let lines: Vec<String> = (0..500)
            .map(|i| format!("- error token-missing h-{i}/tc@0000000000000001: drift"))
            .collect();
        let o = outgoing(
            &DoctorNotice::Baseline {
                at: "t".into(),
                findings: 500,
                checks: 1,
                lines,
                coverage: "coverage: …".into(),
            },
            &rule,
            &RenderConfig {
                max_message_bytes: 512,
                max_structural_bytes: 512,
            },
        );
        assert!(o.notification.truncated);
        assert!(o.notification.message.len() <= 512);
    }
}

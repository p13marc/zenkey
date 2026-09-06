//! The scheduled doctor (#390): the deployment's conformance checks asked
//! every few hours, and only what *changed* told to anyone.
//!
//! `run_doctor`'s stable check ids are properties of a deployment, and
//! until now they were asked in exactly one place — CI, against a fleet
//! stood up ten seconds earlier. The failures that matter are on the fleet
//! that has run for three weeks: a sensor upgraded on four hosts and not
//! the fifth (`slice-sync`, `schema-drift`), a producer that stopped
//! answering `introspect` (`introspect-coverage`), traffic on keys nothing
//! registered. This module runs the same doctor on an interval and turns
//! each run into notices for the engine's one `Notice` stream — the same
//! [`route`], discipline and sinks as everything else.
//!
//! [`route`]: crate::engine::route
//!
//! **A finding true since deployment is not news.** The first run is the
//! **baseline**: one `info` notification counting what it found, and every
//! finding entered into the discipline's ledger *assumed* — announced
//! without a delivery ([`crate::discipline::NoticeMeta::assume`]) — so that
//! the run which fixes one can say `resolved`. Every later run is judged
//! against the previous one by the engine's [`doctor_delta`]: one
//! notification per **new** finding, one `resolved` per **fixed** one, and
//! nothing at all when the two sets are empty. Findings are identified by
//! `(check, subject)`, which is the notice identity — evidence drift is
//! the same finding.
//!
//! **The four poles ride through** (RFC 13 §1). A run that could not
//! happen — the transport, the timeout — is an `unobservable` notification
//! naming the error, and the previous report is **retained**, never
//! replaced with nothing; the run after it is `observable_again`. Inside a
//! report, what was *not asked* is stated, never read as clean: the
//! registry diff that never ran because no registry was loaded (`synced:
//! not asked`), the deep checks that were not requested, the listen phase
//! that did not run — each is a line in every notification and rides the
//! published report exactly as `zenctl doctor --format json` prints it.
//!
//! **Published.** After every run, [`ZenwatchDoctor`] goes out on
//! `state/zenwatch/doctor` (registry `ttl_s = 0`: retained, last writer
//! wins), so the last report is inspectable without SSH and a `next_at` in
//! the past is a schedule that lapsed. **Hours, not seconds**: a run is a
//! fan-in sweep, and it runs beside the drain, never on it.

use std::collections::BTreeMap;
use std::time::Duration;

use zenkey_fleet::report::{Asked, CheckId, DoctorFinding, DoctorReport, DoctorSeverity};
use zenkey_fleet::{CondState, DoctorSpec, RenderSource, doctor_delta};

use crate::config::DoctorConfig;
use crate::discipline::severity_rank;
use crate::publish::{DoctorOutcome, DoctorStatus, ZenwatchDoctor};
use crate::render::{RenderConfig, state_word, truncate_bytes};
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
    match s {
        DoctorSeverity::Error => "error",
        DoctorSeverity::Warning => "warning",
        DoctorSeverity::Info => "info",
    }
}

/// The notice identity of a finding: `doctor:<check>:<subject>` — the
/// `(check, subject)` key [`doctor_delta`] uses, under the rule's id.
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

/// What the run covered, with every not-asked pole said out loud (RFC 13
/// §1, §3 O4): the registry diff that never ran is "not asked", never
/// "0 in sync"; the deep checks and the listen phase likewise; a listen
/// window with drops is unobservable in that span (O6).
pub fn coverage(r: &DoctorReport) -> String {
    let synced = match &r.synced {
        Asked::NotAsked => "not asked (no registry loaded)".to_string(),
        Asked::Asked(v) => format!("asked, {} producer(s) in sync", v.len()),
    };
    let listen = match &r.observation {
        None => "not asked".to_string(),
        Some(o) => format!(
            "{}s, {} sample(s) on {} key(s), {} dropped{}",
            o.window_s,
            o.samples,
            o.keys_seen,
            o.dropped,
            if o.dropped > 0 {
                " — unobservable in that span (O6)"
            } else {
                ""
            }
        ),
    };
    format!(
        "coverage: {} live producer(s), {} introspect answered, describe served {} / \
         missing {}, {} router(s); registry diff: {synced}; deep checks: {}; listen phase: \
         {listen}",
        r.live_producers,
        r.introspect_answered,
        r.describe_served,
        r.describe_missing,
        r.routers,
        if r.deep { "ran" } else { "not asked" },
    )
}

fn rfc3339(now: f64) -> String {
    zenkey_fleet::rfc3339_from_unix(now.max(0.0) as u64)
}

/// The last successful run, kept whole: what the next one is judged
/// against, and what a failed one retains.
#[derive(Debug, Clone)]
struct Previous {
    report: DoctorReport,
    at: String,
}

/// The schedule's state between runs: the previous report, whether the
/// last run failed, and the document last published.
#[derive(Debug)]
pub struct Schedule {
    cfg: DoctorConfig,
    every: Duration,
    timeout: Duration,
    prev: Option<Previous>,
    /// The error of the last run, while it stands.
    failed: Option<String>,
    runs: u64,
    last: Option<ZenwatchDoctor>,
}

impl Schedule {
    /// From a checked config: `every()` is `Some` because [`crate::config::check`]
    /// refused anything else. `timeout` is the bus's, unless the block says.
    pub fn new(cfg: &DoctorConfig, bus_timeout: Duration) -> Schedule {
        Schedule {
            every: cfg.every().unwrap_or(Duration::from_secs(3600)),
            timeout: cfg
                .timeout_s
                .map(Duration::from_secs_f64)
                .unwrap_or(bus_timeout),
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

    /// What one run should cost — the block's `deep`, `sample`, timeout;
    /// no listen phase (a passive window on a scheduled sweep is a watchdog
    /// rule's job, and the `--for` checks are theirs).
    pub fn spec(&self) -> DoctorSpec {
        DoctorSpec {
            deep: self.cfg.deep,
            sample: self.cfg.sample,
            timeout: self.timeout,
            listen: None,
        }
    }

    pub fn runs(&self) -> u64 {
        self.runs
    }

    /// The document last published, if a run has happened.
    pub fn document(&self) -> Option<&ZenwatchDoctor> {
        self.last.as_ref()
    }

    /// For the health document: not yet run, the last run ok, or failed.
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
    /// entry remembered firing which this run does not find is resolved
    /// rather than left announced forever, and a remembered failed run is
    /// `observable_again` on the first success.
    pub fn observe(
        &mut self,
        outcome: Result<DoctorReport, String>,
        now: f64,
        announced: &[String],
    ) -> Vec<DoctorNotice> {
        self.runs += 1;
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
                    findings: self.prev.as_ref().map_or(0, |p| p.report.findings.len()),
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
        let (new, fixed, delta) = match &self.prev {
            None => {
                let checks = report
                    .findings
                    .iter()
                    .map(|f| f.check)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len();
                out.push(DoctorNotice::Baseline {
                    at: ran_at.clone(),
                    findings: report.findings.len(),
                    checks,
                    lines: report
                        .findings
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
                for f in report.findings.iter().filter(|f| self.eligible(f)) {
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
                // while nobody watched — resolved, not announced forever.
                for id in announced {
                    let Some((check, subject)) = id_parts(id) else {
                        continue;
                    };
                    if report
                        .findings
                        .iter()
                        .any(|f| f.check == check && f.subject == subject)
                    {
                        continue;
                    }
                    out.push(DoctorNotice::Finding {
                        finding: DoctorFinding {
                            severity: DoctorSeverity::Info,
                            check,
                            subject: subject.to_string(),
                            evidence: "remembered firing by the state file; absent from the \
                                       first run after the restart"
                                .into(),
                            citation: None,
                        },
                        state: CondState::Ok,
                        prior: Some(CondState::Firing),
                        assume: false,
                        since_run: None,
                        coverage: cov.clone(),
                    });
                }
                (0, 0, None)
            }
            Some(prev) => {
                let d = doctor_delta(&prev.report, &report);
                for f in d.new.iter().filter(|f| self.eligible(f)) {
                    out.push(DoctorNotice::Finding {
                        finding: f.clone(),
                        state: CondState::Firing,
                        prior: Some(CondState::Ok),
                        assume: false,
                        since_run: Some(prev.at.clone()),
                        coverage: cov.clone(),
                    });
                }
                for f in d.fixed.iter().filter(|f| self.eligible(f)) {
                    out.push(DoctorNotice::Finding {
                        finding: f.clone(),
                        state: CondState::Ok,
                        prior: Some(CondState::Firing),
                        assume: false,
                        since_run: Some(prev.at.clone()),
                        coverage: cov.clone(),
                    });
                }
                let (n, x) = (d.new.len(), d.fixed.len());
                (
                    n,
                    x,
                    Some(serde_json::to_value(&d).expect("a delta serializes")),
                )
            }
        };
        self.last = Some(ZenwatchDoctor {
            ran_at: ran_at.clone(),
            next_at,
            every_s,
            outcome: DoctorOutcome::Ok,
            error: None,
            findings: report.findings.len(),
            new,
            fixed,
            report: serde_json::to_value(&report).expect("a report serializes"),
            report_at: Some(ran_at.clone()),
            delta,
        });
        self.prev = Some(Previous { report, at: ran_at });
        out
    }
}

/// One doctor notice as the [`Outgoing`] the scheduled rule routes —
/// rendered here rather than in [`crate::render`] because a doctor finding
/// is a judgement over a sweep, not a payload on a key: the message is the
/// check, the subject, the evidence, the citation and the coverage line,
/// bounded by the same byte limit.
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
            let check_line = match &finding.citation {
                Some(c) => format!("check: {} ({c})", finding.check),
                None => format!("check: {}", finding.check),
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
                    check_line,
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
                    "the previous report is retained on state/zenwatch/doctor; nothing here \
                     says the deployment is clean (RFC 13 §1)"
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
            inhibited_by: None,
        },
        sinks: rule.sinks.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DisciplineConfig;
    use crate::discipline::{Discipline, NoticeMeta};
    use std::collections::BTreeSet;

    fn finding(check: CheckId, subject: &str, severity: DoctorSeverity) -> DoctorFinding {
        DoctorFinding {
            severity,
            check,
            subject: subject.into(),
            evidence: format!("evidence for {subject}"),
            citation: Some("RFC 08 §6".into()),
        }
    }

    fn report(findings: Vec<DoctorFinding>, synced: Asked<Vec<String>>) -> DoctorReport {
        DoctorReport {
            findings,
            synced,
            introspect_answered: 3,
            live_producers: 3,
            describe_served: 2,
            describe_missing: 1,
            routers: 0,
            router_version: None,
            deep: false,
            observation: None,
        }
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
                schedule: Schedule::new(&cfg, Duration::from_secs(2)),
                rule,
                discipline,
                render: RenderConfig::default(),
            }
        }

        fn run(&mut self, outcome: Result<DoctorReport, String>, now: f64) -> Vec<Outgoing> {
            let announced = self.discipline.announced_under(DOCTOR_RULE);
            for n in self.schedule.observe(outcome, now, &announced) {
                let meta = NoticeMeta {
                    origin: None,
                    passthrough: n.is_passthrough(),
                    assume: n.is_assumed(),
                };
                let o = outgoing(&n, &self.rule, &self.render);
                self.discipline.observe(o, &meta, now);
            }
            self.discipline
                .tick(now, None, &BTreeSet::new(), false)
                .outgoing
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

    /// Baseline, then deltas: the first run is one `info` notification
    /// counting the findings and naming them, not one per finding; an
    /// identical second run is nothing; a third with one new and one fixed
    /// is one `doctor` firing and one `resolved`, each naming its check and
    /// subject; and the published document counts each step.
    #[test]
    fn a_baseline_then_only_deltas() {
        let mut h = Harness::new(None);
        let a = finding(CheckId::SliceSync, "h-1/sysinfo", DoctorSeverity::Error);
        let b = finding(CheckId::DescribeMissing, "fleet", DoctorSeverity::Info);
        let out = h.run(
            Ok(report(vec![a.clone(), b.clone()], Asked::Asked(vec![]))),
            0.0,
        );
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
            n.message
                .contains("- error slice-sync h-1/sysinfo: evidence for h-1/sysinfo"),
            "{}",
            n.message
        );
        assert!(
            n.message
                .contains("registry diff: asked, 0 producer(s) in sync")
        );
        assert_eq!(out[0].sinks, vec!["ops".to_string()]);
        // Both findings are remembered firing under the rule, delivered to nobody.
        assert_eq!(
            h.discipline.announced_under(DOCTOR_RULE),
            vec![
                "doctor:describe-missing:fleet".to_string(),
                "doctor:slice-sync:h-1/sysinfo".to_string()
            ]
        );
        let doc = h.schedule.document().unwrap().clone();
        assert_eq!(doc.outcome, DoctorOutcome::Ok);
        assert_eq!((doc.findings, doc.new, doc.fixed), (2, 0, 0));
        assert_eq!(doc.delta, None);
        assert_eq!(doc.every_s, 6.0 * 3600.0);
        assert_eq!(doc.report["synced"], serde_json::json!([]));
        assert_eq!(doc.report["findings"][0]["check"], "slice-sync");
        assert_eq!(doc.report_at.as_deref(), Some(doc.ran_at.as_str()));

        // The same again: nothing.
        let out = h.run(
            Ok(report(vec![a.clone(), b.clone()], Asked::Asked(vec![]))),
            100.0,
        );
        assert!(out.is_empty(), "{out:?}");
        let doc = h.schedule.document().unwrap();
        assert_eq!((doc.findings, doc.new, doc.fixed), (2, 0, 0));
        assert_eq!(doc.delta.as_ref().unwrap()["unchanged"], 2);

        // A drifts to a different wording (same finding), B is fixed, C is new.
        let mut a2 = a.clone();
        a2.evidence = "different wording".into();
        let c = finding(
            CheckId::SchemaDrift,
            "TelemetryPoint",
            DoctorSeverity::Error,
        );
        let out = h.run(Ok(report(vec![a2, c], Asked::Asked(vec![]))), 200.0);
        assert_eq!(
            ids(&out),
            vec![
                (
                    "doctor:describe-missing:fleet".to_string(),
                    NoticeKind::Resolved,
                    CondState::Ok
                ),
                (
                    "doctor:schema-drift:TelemetryPoint".to_string(),
                    NoticeKind::Doctor,
                    CondState::Firing
                ),
            ]
        );
        let new = &out[1].notification;
        assert_eq!(new.title, "schema-drift TelemetryPoint");
        assert_eq!(new.severity, "error");
        assert_eq!(new.prior, Some(CondState::Ok));
        assert_eq!(
            new.labels.get("check").map(String::as_str),
            Some("schema-drift")
        );
        assert!(
            new.message.contains("check: schema-drift (RFC 08 §6)"),
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
                .contains("evidence: evidence for TelemetryPoint")
        );
        let fixed = &out[0].notification;
        assert_eq!(fixed.title, "describe-missing fleet");
        assert_eq!(fixed.prior, Some(CondState::Firing));
        let doc = h.schedule.document().unwrap();
        assert_eq!((doc.findings, doc.new, doc.fixed), (2, 1, 1));
        assert_eq!(
            doc.delta.as_ref().unwrap()["new"][0]["check"],
            "schema-drift"
        );
        assert_eq!(
            doc.delta.as_ref().unwrap()["fixed"][0]["check"],
            "describe-missing"
        );
        assert_eq!(h.schedule.runs(), 3);
    }

    /// A run that fails is `unobservable`, naming the error, with the
    /// previous report retained in the document (and its findings still
    /// counted); the run after it is `observable_again`, and its findings
    /// are judged against the last report that succeeded — not against
    /// nothing.
    #[test]
    fn a_failed_run_is_unobservable_and_keeps_the_baseline() {
        let mut h = Harness::new(None);
        let a = finding(CheckId::SliceSync, "h-1/sysinfo", DoctorSeverity::Error);
        h.run(Ok(report(vec![a.clone()], Asked::NotAsked)), 0.0);
        assert_eq!(h.schedule.status(), DoctorStatus::Ok);

        let out = h.run(Err("timed out asking the roster".into()), 100.0);
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
            n.message.contains("timed out asking the roster"),
            "{}",
            n.message
        );
        assert!(n.message.contains("retained"), "{}", n.message);
        assert_eq!(h.schedule.status(), DoctorStatus::Failed);
        let doc = h.schedule.document().unwrap().clone();
        assert_eq!(doc.outcome, DoctorOutcome::Failed);
        assert_eq!(doc.error.as_deref(), Some("timed out asking the roster"));
        assert_eq!(doc.findings, 1, "the baseline's findings, retained");
        assert_eq!(doc.report["findings"][0]["subject"], "h-1/sysinfo");
        assert_ne!(doc.report_at, Some(doc.ran_at.clone()), "an older report");
        assert_eq!(doc.delta, None);
        // Still remembered firing.
        assert_eq!(h.discipline.announced_under(DOCTOR_RULE).len(), 2);

        // A second failure is a duplicate, not a second page.
        assert!(h.run(Err("still down".into()), 150.0).is_empty());

        // Recovery: observable again, and the baseline finding is now
        // fixed — judged against the run at 0, not against nothing.
        let out = h.run(Ok(report(vec![], Asked::NotAsked)), 200.0);
        assert_eq!(
            ids(&out),
            vec![
                (
                    RUN_ID.to_string(),
                    NoticeKind::ObservableAgain,
                    CondState::Ok
                ),
                (
                    "doctor:slice-sync:h-1/sysinfo".to_string(),
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
        let out = h.run(Ok(report(vec![], Asked::NotAsked)), 100.0);
        assert_eq!(
            ids(&out).iter().map(|(_, k, _)| *k).collect::<Vec<_>>(),
            vec![NoticeKind::Doctor, NoticeKind::ObservableAgain],
            "flushed in id order: the baseline, then the run"
        );
        assert_eq!(out[0].notification.state, CondState::Ok, "a clean baseline");
    }

    /// `synced: not asked` (no registry) is carried as-is — absent from the
    /// published report, exactly as `zenctl doctor --format json` prints
    /// it, and stated as "not asked" in every message — never as clean;
    /// the deep checks and the listen phase likewise.
    #[test]
    fn not_asked_rides_through_never_as_clean() {
        let mut h = Harness::new(None);
        let out = h.run(Ok(report(vec![], Asked::NotAsked)), 0.0);
        let n = &out[0].notification;
        assert_eq!(n.state, CondState::Ok);
        assert!(
            n.message
                .contains("registry diff: not asked (no registry loaded)"),
            "{}",
            n.message
        );
        assert!(n.message.contains("deep checks: not asked"));
        assert!(n.message.contains("listen phase: not asked"));
        let doc = h.schedule.document().unwrap();
        assert!(
            !doc.report.as_object().unwrap().contains_key("synced"),
            "not asked is absence on the wire, as the report contract pins: {}",
            doc.report
        );
        let asked = report(
            vec![],
            Asked::Asked(vec!["h-1/sysinfo (registry 1.0)".into()]),
        );
        let deep = DoctorReport {
            deep: true,
            ..asked
        };
        assert!(coverage(&deep).contains("asked, 1 producer(s) in sync"));
        assert!(coverage(&deep).contains("deep checks: ran"));
        // A finding's message carries the same line.
        let mut h = Harness::new(None);
        h.run(Ok(report(vec![], Asked::NotAsked)), 0.0);
        let out = h.run(
            Ok(report(
                vec![finding(
                    CheckId::StaleState,
                    "v1/h-1/state/p/health",
                    DoctorSeverity::Warning,
                )],
                Asked::NotAsked,
            )),
            100.0,
        );
        assert_eq!(out.len(), 1);
        assert!(
            out[0]
                .notification
                .message
                .contains("registry diff: not asked")
        );
    }

    /// The severity floor: an `info` finding below a `warning` floor is in
    /// the report and its counts but is never a notification — new or
    /// fixed; the baseline still counts it.
    #[test]
    fn the_severity_floor_keeps_low_findings_in_the_report_only() {
        let mut h = Harness::new(Some("warning"));
        let info = finding(CheckId::DescribeMissing, "fleet", DoctorSeverity::Info);
        let out = h.run(Ok(report(vec![info.clone()], Asked::NotAsked)), 0.0);
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
            CheckId::UnregisteredTraffic,
            "v1/h-1/x",
            DoctorSeverity::Warning,
        );
        let out = h.run(Ok(report(vec![warn.clone()], Asked::NotAsked)), 100.0);
        assert_eq!(out.len(), 1, "the warning, not the fixed info: {out:?}");
        assert_eq!(
            out[0].notification.id,
            "doctor:unregistered-traffic:v1/h-1/x"
        );
        let doc = h.schedule.document().unwrap();
        assert_eq!(
            (doc.findings, doc.new, doc.fixed),
            (1, 1, 1),
            "the document counts everything"
        );
    }

    /// After a restart the state file remembers what was announced: a
    /// finding still present is assumed silently, one gone is resolved,
    /// and a remembered failed run is observable again.
    #[test]
    fn a_restart_reconciles_the_baseline_against_the_ledger() {
        let mut h = Harness::new(None);
        // Stand in for the restored ledger: two findings announced, and
        // the run itself unobservable.
        let present = finding(CheckId::SliceSync, "h-1/sysinfo", DoctorSeverity::Error);
        let gone = finding(
            CheckId::SchemaDrift,
            "TelemetryPoint",
            DoctorSeverity::Error,
        );
        for f in [&present, &gone] {
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
        h.discipline.tick(-100.0, None, &BTreeSet::new(), false);
        assert_eq!(h.discipline.announced_under(DOCTOR_RULE).len(), 3);

        let out = h.run(Ok(report(vec![present], Asked::NotAsked)), 0.0);
        assert_eq!(
            ids(&out),
            vec![
                (
                    BASELINE_ID.to_string(),
                    NoticeKind::Doctor,
                    CondState::Firing
                ),
                (
                    RUN_ID.to_string(),
                    NoticeKind::ObservableAgain,
                    CondState::Ok
                ),
                (
                    "doctor:schema-drift:TelemetryPoint".to_string(),
                    NoticeKind::Resolved,
                    CondState::Ok
                ),
            ]
        );
        assert_eq!(
            h.discipline.announced_under(DOCTOR_RULE),
            vec!["doctor:slice-sync:h-1/sysinfo".to_string()]
        );
        assert_eq!(
            id_parts("doctor:slice-sync:h-1/sysinfo:with:colons"),
            Some((CheckId::SliceSync, "h-1/sysinfo:with:colons"))
        );
        assert_eq!(id_parts("doctor:run"), None);
        assert_eq!(id_parts("fleet-alerts:x"), None);
    }

    /// The message is bounded like every rendering: a baseline with many
    /// findings is cut at the byte limit and says so.
    #[test]
    fn the_baseline_is_bounded() {
        let cfg = cfg(None);
        let rule = Rule::scheduled_doctor(&cfg);
        let lines: Vec<String> = (0..500)
            .map(|i| format!("- error slice-sync h-{i}/sysinfo: drift"))
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

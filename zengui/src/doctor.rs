//! The doctor panel's model (#71): run state, run-over-run deltas, and
//! finding navigation — pure logic over the engine's `DoctorReport`, which
//! is the *same struct* `zenctl doctor --format json` serializes. The panel
//! never reimplements a check (RFC 08 §6.1's argument: only a check catches
//! a lying registry, and it must not live in only one frontend).

use std::collections::BTreeSet;
use std::sync::Arc;

use zenkey_fleet::report::{DoctorFinding, DoctorReport};

/// Findings are identified by `(check id, subject)` for delta purposes;
/// evidence and severity drift count as "unchanged" — the *fact* persists,
/// its wording may move.
pub type FindingKey = (String, String);

fn key_of(f: &DoctorFinding) -> FindingKey {
    (f.check.clone(), f.subject.clone())
}

/// One run-over-run delta.
#[derive(Debug, Default)]
pub struct Delta {
    /// Findings present in the previous run and gone now.
    pub fixed: Vec<DoctorFinding>,
    /// Keys of findings new in the current run.
    pub new: BTreeSet<FindingKey>,
    /// Count of findings present in both runs.
    pub unchanged: usize,
}

pub fn delta(previous: &DoctorReport, current: &DoctorReport) -> Delta {
    let cur_keys: BTreeSet<FindingKey> = current.findings.iter().map(key_of).collect();
    let prev_keys: BTreeSet<FindingKey> = previous.findings.iter().map(key_of).collect();
    Delta {
        fixed: previous
            .findings
            .iter()
            .filter(|f| !cur_keys.contains(&key_of(f)))
            .cloned()
            .collect(),
        new: cur_keys.difference(&prev_keys).cloned().collect(),
        unchanged: cur_keys.intersection(&prev_keys).count(),
    }
}

/// One finished run and the base it was judged against — the staleness
/// guard (#109). `DoctorReport` is the engine's serde type, shared with
/// `zenctl doctor --format json`, so the base rides *beside* the report
/// rather than inside it.
#[derive(Debug, Clone)]
pub struct DoctorRun {
    pub report: Arc<DoctorReport>,
    pub base: String,
}

/// The panel's whole state.
#[derive(Debug, Default)]
pub struct DoctorState {
    pub in_flight: bool,
    pub deep: bool,
    pub current: Option<Arc<DoctorReport>>,
    /// The last *successful* run before `current` — a failed run keeps the
    /// baseline (deltas against nothing would misreport everything as new).
    pub previous: Option<Arc<DoctorReport>>,
    pub delta: Option<Delta>,
    pub error: Option<String>,
    /// How many times the schema cache has been cleared this session
    /// (issue #101) — shown so the button is visibly a thing that happened,
    /// not a no-op the user has to guess at.
    pub schemas_forgotten: usize,
}

impl DoctorState {
    /// A run finished. Shifts current → previous only on success — and only
    /// when the run was judged against the base this window is looking at.
    ///
    /// A run from another base is dropped silently, deliberately (#109,
    /// mirroring `AdminState::finish`): it is not an error, and reporting it
    /// would explain a deployment the user has left. `clear()` cannot stop a
    /// task already in flight, and a doctor run takes seconds — this is what
    /// catches the one that lands after the switch. The `Err` arm is *not*
    /// base-guarded, same asymmetry as admin: an error carries no base to
    /// judge, and it rides the Ok carrier only.
    pub fn finish(&mut self, outcome: Result<DoctorRun, String>, base: &str) {
        self.in_flight = false;
        match outcome {
            Ok(run) if run.base == base => {
                self.delta = self.current.as_deref().map(|prev| delta(prev, &run.report));
                self.previous = self.current.replace(run.report);
                self.error = None;
            }
            Ok(_) => {
                // A run about another deployment says nothing about this one.
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Base change / reconnect: a report against another base is not
    /// evidence here.
    pub fn clear(&mut self) {
        *self = DoctorState {
            deep: self.deep,
            ..DoctorState::default()
        };
    }
}

/// Where a finding can navigate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A concrete wire key — select it in the tree (and fetch its value).
    Key(String),
    /// An `origin/producer` subject — select the origin in the nodes pane.
    Node(String),
}

/// Resolve a finding's subject to a navigation target. `fleet` / `mesh`
/// subjects (and anything else unresolvable) navigate nowhere — the button
/// simply is not rendered.
pub fn finding_target(finding: &DoctorFinding, base: &str) -> Option<Target> {
    let subject = &finding.subject;
    if subject == "fleet" || subject == "mesh" {
        return None;
    }
    // A concrete wire key parses under the configured base.
    if zenkey::grammar::parse_full(base, subject).is_some() {
        return Some(Target::Key(subject.clone()));
    }
    // `origin/producer` — the shape slice-sync/slice-parse findings use.
    if let Some((origin, _producer)) = subject.split_once('/')
        && (zenkey::grammar::is_valid_host_origin(origin) || origin.starts_with('@'))
    {
        return Some(Target::Node(origin.to_string()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_fleet::report::DoctorSeverity;

    fn finding(check: &str, subject: &str) -> DoctorFinding {
        DoctorFinding {
            severity: DoctorSeverity::Error,
            check: check.into(),
            subject: subject.into(),
            evidence: "e".into(),
            citation: None,
        }
    }

    fn report(findings: Vec<DoctorFinding>) -> DoctorReport {
        DoctorReport {
            findings,
            synced: vec![],
            introspect_answered: 0,
            live_producers: 0,
            describe_served: 0,
            describe_missing: 0,
            routers: 0,
            router_version: None,
            deep: false,
        }
    }

    #[test]
    fn deltas_key_on_check_and_subject() {
        let prev = report(vec![
            finding("slice-sync", "h-1/sysinfo"),
            finding("stale-state", "v1/h-1/state/p/health"),
        ]);
        let mut changed = finding("slice-sync", "h-1/sysinfo");
        changed.evidence = "different wording".into();
        let cur = report(vec![changed, finding("schema-drift", "TelemetryPoint")]);

        let d = delta(&prev, &cur);
        assert_eq!(d.unchanged, 1, "evidence drift is still the same finding");
        assert_eq!(d.fixed.len(), 1);
        assert_eq!(d.fixed[0].check, "stale-state");
        assert!(
            d.new
                .contains(&("schema-drift".to_string(), "TelemetryPoint".to_string()))
        );
    }

    fn run(base: &str, findings: Vec<DoctorFinding>) -> DoctorRun {
        DoctorRun {
            report: Arc::new(report(findings)),
            base: base.into(),
        }
    }

    #[test]
    fn a_failed_run_keeps_the_baseline() {
        let mut state = DoctorState::default();
        state.finish(
            Ok(run("acme", vec![finding("slice-sync", "h-1/p")])),
            "acme",
        );
        assert!(state.current.is_some());

        state.finish(Err("bus went away".into()), "acme");
        assert!(state.current.is_some(), "the baseline survives a failure");
        assert!(state.error.is_some());
    }

    /// #109: the user switched deployment while a run was in flight. Its
    /// findings are about a deployment this window no longer shows — dropped,
    /// not displayed, and not an error either (the shape of
    /// `admin.rs::a_sweep_from_another_base_is_not_evidence_here`).
    #[test]
    fn a_run_from_another_base_is_not_evidence_here() {
        let mut state = DoctorState::default();
        state.finish(
            Ok(run("acme", vec![finding("slice-sync", "h-1/p")])),
            "acme",
        );
        assert!(state.current.is_some());

        state.clear();
        state.finish(Ok(run("acme", vec![finding("stale-state", "k")])), "other");
        assert!(
            state.current.is_none(),
            "a run about acme says nothing about other"
        );
        assert!(state.delta.is_none());
        assert!(state.error.is_none(), "and it is not an error either");
    }

    #[test]
    fn finding_targets_resolve_by_subject_shape() {
        assert_eq!(finding_target(&finding("x", "fleet"), ""), None);
        assert_eq!(finding_target(&finding("x", "mesh"), ""), None);
        assert_eq!(
            finding_target(
                &finding("stale-state", "v1/h-3fa9c2d41b7e/state/p/health"),
                ""
            ),
            Some(Target::Key("v1/h-3fa9c2d41b7e/state/p/health".into()))
        );
        assert_eq!(
            finding_target(&finding("slice-sync", "h-3fa9c2d41b7e/sysinfo"), ""),
            Some(Target::Node("h-3fa9c2d41b7e".into()))
        );
        assert_eq!(
            finding_target(&finding("schema-drift", "TelemetryPoint"), ""),
            None,
            "a bare type name navigates nowhere"
        );
    }
}

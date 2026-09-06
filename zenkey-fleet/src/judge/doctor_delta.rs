//! Run-over-run doctor deltas (#389): what a second doctor run says
//! *relative to* the first. Moved from zengui's doctor panel, because a
//! notifier reporting "the doctor found something new" and a pane
//! rendering "fixed since last run" must agree on what *new* means.
//!
//! A finding is identified by `(check, subject)`: evidence and severity
//! drift count as unchanged — the *fact* persists, its wording may move.
//! Ungated on purpose: the doctor itself needs a session and the `decode`
//! feature, but comparing two of its reports needs neither.

use std::collections::BTreeSet;

use crate::report::{CheckId, DoctorDelta, DoctorFinding, DoctorReport};

fn key_of(f: &DoctorFinding) -> (CheckId, &str) {
    (f.check, f.subject.as_str())
}

/// What `current` says that `previous` did not, and the other way round.
pub fn doctor_delta(previous: &DoctorReport, current: &DoctorReport) -> DoctorDelta {
    let cur_keys: BTreeSet<(CheckId, &str)> = current.findings.iter().map(key_of).collect();
    let prev_keys: BTreeSet<(CheckId, &str)> = previous.findings.iter().map(key_of).collect();
    DoctorDelta {
        new: current
            .findings
            .iter()
            .filter(|f| !prev_keys.contains(&key_of(f)))
            .cloned()
            .collect(),
        fixed: previous
            .findings
            .iter()
            .filter(|f| !cur_keys.contains(&key_of(f)))
            .cloned()
            .collect(),
        unchanged: cur_keys.intersection(&prev_keys).count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Asked, DoctorSeverity};

    fn finding(check: CheckId, subject: &str) -> DoctorFinding {
        DoctorFinding {
            severity: DoctorSeverity::Error,
            check,
            subject: subject.into(),
            evidence: "e".into(),
            citation: None,
        }
    }

    fn report(findings: Vec<DoctorFinding>) -> DoctorReport {
        DoctorReport {
            findings,
            synced: Asked::NotAsked,
            introspect_answered: 0,
            live_producers: 0,
            describe_served: 0,
            describe_missing: 0,
            routers: 0,
            router_version: None,
            deep: false,
            observation: None,
        }
    }

    /// Ported from zengui: keyed on `(check, subject)`, evidence drift is
    /// still the same finding.
    #[test]
    fn deltas_key_on_check_and_subject() {
        let prev = report(vec![
            finding(CheckId::SliceSync, "h-1/sysinfo"),
            finding(CheckId::StaleState, "v1/h-1/state/p/health"),
        ]);
        let mut changed = finding(CheckId::SliceSync, "h-1/sysinfo");
        changed.evidence = "different wording".into();
        let cur = report(vec![
            changed,
            finding(CheckId::SchemaDrift, "TelemetryPoint"),
        ]);

        let d = doctor_delta(&prev, &cur);
        assert_eq!(d.unchanged, 1, "evidence drift is still the same finding");
        assert_eq!(d.fixed.len(), 1);
        assert_eq!(d.fixed[0].check, CheckId::StaleState);
        assert_eq!(d.new.len(), 1);
        assert!(d.is_new(&finding(CheckId::SchemaDrift, "TelemetryPoint")));
        assert!(!d.is_new(&finding(CheckId::SliceSync, "h-1/sysinfo")));
    }

    /// Two empty runs: nothing new, nothing fixed, nothing unchanged.
    #[test]
    fn two_clean_runs_differ_in_nothing() {
        let d = doctor_delta(&report(vec![]), &report(vec![]));
        assert!(d.new.is_empty() && d.fixed.is_empty());
        assert_eq!(d.unchanged, 0);
    }
}

//! Run-over-run doctor deltas (#389): what a second doctor run says
//! *relative to* the first. Moved from zengui's doctor panel, because a
//! notifier reporting "the doctor found something new" and a pane
//! rendering "fixed since last run" must agree on what *new* means.
//!
//! A finding is identified by `(check, subject)`: evidence and severity
//! drift count as unchanged — the *fact* persists, its wording may move.
//! zk2's checks (#612, FJ6) spell every subject the same way every run for
//! exactly this reason. Ungated on purpose: the doctor needs a session, but
//! comparing two of its reports needs none.

use std::collections::BTreeSet;

use crate::report::{CheckId, DoctorDelta, DoctorFinding, DoctorReport};

fn key_of(f: &DoctorFinding) -> (CheckId, &str) {
    (f.check, f.subject.as_str())
}

/// What `current` says that `previous` did not, and the other way round.
///
/// A check that was not established in one of the runs — not asked, or
/// unobservable — has no findings there, so its findings read as new or
/// fixed against it. A notifier that must not announce a fix nobody saw
/// reads the check's verdict beside the delta, as zenwatch's scheduled
/// doctor does.
pub fn doctor_delta(previous: &DoctorReport, current: &DoctorReport) -> DoctorDelta {
    let cur_keys: BTreeSet<(CheckId, &str)> = current.findings().map(key_of).collect();
    let prev_keys: BTreeSet<(CheckId, &str)> = previous.findings().map(key_of).collect();
    DoctorDelta {
        new: current
            .findings()
            .filter(|f| !prev_keys.contains(&key_of(f)))
            .cloned()
            .collect(),
        fixed: previous
            .findings()
            .filter(|f| !cur_keys.contains(&key_of(f)))
            .cloned()
            .collect(),
        unchanged: cur_keys.intersection(&prev_keys).count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Asked, CheckReport, DoctorScope, DoctorSeverity};

    fn finding(check: CheckId, subject: &str) -> DoctorFinding {
        DoctorFinding {
            severity: DoctorSeverity::Error,
            check,
            subject: subject.into(),
            evidence: "e".into(),
        }
    }

    /// A report whose checks carry `findings`, each under its own check.
    fn report(findings: Vec<DoctorFinding>) -> DoctorReport {
        let checks = CheckId::ALL
            .into_iter()
            .map(|id| {
                let mine: Vec<DoctorFinding> =
                    findings.iter().filter(|f| f.check == id).cloned().collect();
                CheckReport::of(id, mine, vec![], "clean")
            })
            .collect();
        DoctorReport {
            scope: DoctorScope {
                namespace: String::new(),
                presence: Asked::NotAsked,
                routers: Asked::NotAsked,
                health: Asked::NotAsked,
            },
            checks,
            unobservable: None,
        }
    }

    /// Keyed on `(check, subject)` over zk2's check ids: evidence drift is
    /// still the same finding, and one subject under two checks is two.
    #[test]
    fn deltas_key_on_check_and_subject() {
        let prev = report(vec![
            finding(CheckId::SplitBrain, "h1/tc tc.v1"),
            finding(CheckId::ContractUnavailable, "tc.v1 sha256:00"),
        ]);
        let mut changed = finding(CheckId::SplitBrain, "h1/tc tc.v1");
        changed.evidence = "different wording".into();
        let cur = report(vec![
            changed,
            finding(CheckId::TokenMissing, "h1/tc@0000000000000001 tc.v1"),
            finding(CheckId::DescriptorInvalid, "h1/tc tc.v1"),
        ]);

        let d = doctor_delta(&prev, &cur);
        assert_eq!(d.unchanged, 1, "evidence drift is still the same finding");
        assert_eq!(d.fixed.len(), 1);
        assert_eq!(d.fixed[0].check, CheckId::ContractUnavailable);
        assert_eq!(d.new.len(), 2);
        assert!(d.is_new(&finding(
            CheckId::TokenMissing,
            "h1/tc@0000000000000001 tc.v1"
        )));
        assert!(d.is_new(&finding(CheckId::DescriptorInvalid, "h1/tc tc.v1")));
        assert!(!d.is_new(&finding(CheckId::SplitBrain, "h1/tc tc.v1")));
    }

    /// Two clean runs: nothing new, nothing fixed, nothing unchanged.
    #[test]
    fn two_clean_runs_differ_in_nothing() {
        let d = doctor_delta(&report(vec![]), &report(vec![]));
        assert!(d.new.is_empty() && d.fixed.is_empty());
        assert_eq!(d.unchanged, 0);
    }
}

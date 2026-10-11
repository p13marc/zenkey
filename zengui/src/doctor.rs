//! The doctor panel's model (#71; zk2's since #614, FL9): run state,
//! run-over-run deltas, and finding navigation — pure logic over the
//! engine's zk2 `DoctorReport`, which is the *same struct* `zenctl doctor
//! --format json` serializes: one `Judgement` per check, each named for a
//! condition firing, its finding the yes. The panel never reimplements a
//! check — only a check catches a deployment lying about itself, and it must
//! not live in only one frontend.

use std::sync::Arc;

use zenkey_fleet::doctor_delta;
use zenkey_fleet::report::{DoctorDelta, DoctorFinding, DoctorReport};
use zenkey_model::grammar::{Addr, ZkKey};

/// One finished run and the namespace it was judged in — the staleness
/// guard (#109). `DoctorReport` is the engine's serde type, shared with
/// `zenctl doctor --format json`; its scope names the namespace too, and
/// this is the one the window asked in.
#[derive(Debug, Clone)]
pub struct DoctorRun {
    pub report: Arc<DoctorReport>,
    pub namespace: String,
}

/// The panel's whole state.
#[derive(Debug, Default)]
pub struct DoctorState {
    pub in_flight: bool,
    /// Ask the checks that read the owners' data plane —
    /// `state-stamp-foreign` and `population-over-bound`, a GET of every
    /// owner's state each (`DoctorSpec::deep`).
    pub deep: bool,
    /// The operator's word that this host's clock and the owners' agree
    /// (`DoctorSpec::clocks_synced`, `freshness.v1` §2.6): without it the
    /// `health-*` checks of every service implementing `health.v1` are
    /// unobservable. Off by default (D11), and its own toggle beside the
    /// nodes pane's, as `zenctl doctor` and `zenctl health` each take
    /// `--clocks-synced`.
    pub clocks_synced: bool,
    pub current: Option<Arc<DoctorReport>>,
    /// The run before `current`.
    pub previous: Option<Arc<DoctorReport>>,
    /// The engine's run-over-run comparison (#389): keyed on `(check,
    /// subject)`, shared with zenwatch's scheduled doctor.
    pub delta: Option<DoctorDelta>,
    /// Why a run could not start — no session to ask through. A run that
    /// started always lands a report: an input it could not read is a
    /// check's `Unobservable`, never an error here.
    pub error: Option<crate::services::ServiceError>,
    /// How many times presence and the contracts have been re-read this
    /// session — shown so the button is visibly a thing that happened, not
    /// a no-op the user has to guess at (#101).
    pub rereads: usize,
}

impl DoctorState {
    /// A run finished. Shifts current → previous — but only when the run
    /// was judged in the namespace this window is looking at.
    ///
    /// A run from another namespace is dropped silently, deliberately
    /// (#109, mirroring `AdminState::finish`): it is not an error, and
    /// reporting it would explain a deployment the user has left. `clear()`
    /// cannot stop a task already in flight, and a doctor run takes seconds
    /// — this is what catches the one that lands after the switch.
    pub fn finish(&mut self, run: DoctorRun, namespace: &str) {
        self.in_flight = false;
        if run.namespace != namespace {
            return;
        }
        self.delta = self
            .current
            .as_deref()
            .map(|prev| doctor_delta(prev, &run.report));
        self.previous = self.current.replace(run.report);
        self.error = None;
    }

    /// Namespace change / reconnect: a report about another deployment is
    /// not evidence here. The two toggles are the operator's, and stay.
    pub fn clear(&mut self) {
        *self = DoctorState {
            deep: self.deep,
            clocks_synced: self.clocks_synced,
            ..DoctorState::default()
        };
    }
}

/// Where a finding can navigate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A concrete zk2 key, as the wire spells it in this namespace — select
    /// it in the tree (and fetch its value).
    Key(String),
    /// A service address — select it, and land on the nodes tool.
    Address(Addr),
}

/// Resolve a finding's subject to a navigation target.
///
/// zk2's checks spell a subject the same way every run (the delta keys on
/// it): `<system>/<service>@<instance>`, `<system>/<service> <role>`,
/// `<system>/<service> <iface>`, `<iface> <fingerprint>`,
/// `<storage>@<zid>`. Its first word is read: a zk2 data key navigates to
/// itself under the namespace, a `<system>/<service>` (an instance id taken
/// off) to the address. A revision, a storage, the presence domain or this
/// host navigates nowhere — the button simply is not rendered.
pub fn finding_target(finding: &DoctorFinding, namespace: &str) -> Option<Target> {
    let first = finding.subject.split_whitespace().next()?;
    if let Ok(ZkKey::Data { .. }) = zenkey_model::grammar::parse(first) {
        return Some(Target::Key(zenkey_fleet::with_namespace(namespace, first)));
    }
    let addr = first.split('@').next()?;
    if !addr.contains('/') {
        return None;
    }
    addr.parse::<Addr>().ok().map(Target::Address)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_fleet::report::{Asked, CheckId, CheckReport, DoctorScope, DoctorSeverity};

    fn finding(check: CheckId, subject: &str) -> DoctorFinding {
        DoctorFinding {
            severity: DoctorSeverity::Error,
            check,
            subject: subject.into(),
            evidence: "e".into(),
        }
    }

    fn report(findings: Vec<DoctorFinding>) -> DoctorReport {
        DoctorReport {
            scope: DoctorScope {
                namespace: "acme".into(),
                presence: Asked::NotAsked,
                routers: Asked::NotAsked,
                health: Asked::NotAsked,
            },
            checks: CheckId::ALL
                .into_iter()
                .map(|id| {
                    let mine: Vec<DoctorFinding> =
                        findings.iter().filter(|f| f.check == id).cloned().collect();
                    CheckReport::of(id, mine, vec![], "clean")
                })
                .collect(),
            unobservable: None,
        }
    }

    fn run(namespace: &str, findings: Vec<DoctorFinding>) -> DoctorRun {
        DoctorRun {
            report: Arc::new(report(findings)),
            namespace: namespace.into(),
        }
    }

    /// The second run is compared with the first, keyed on `(check,
    /// subject)`: the engine's delta, shared with zenwatch.
    #[test]
    fn the_second_run_is_a_delta_against_the_first() {
        let mut state = DoctorState::default();
        let missing = finding(CheckId::TokenMissing, "robot9/phantom@00000000000000aa");
        state.finish(run("acme", vec![missing.clone()]), "acme");
        assert!(state.delta.is_none(), "one run compares with nothing");
        let squatter = finding(CheckId::StorageOnState, "state@z1");
        state.finish(run("acme", vec![squatter.clone()]), "acme");
        let d = state.delta.as_ref().expect("a delta");
        assert!(d.is_new(&squatter));
        assert_eq!(d.fixed, vec![missing]);
        assert!(state.previous.is_some());
    }

    /// #109: the user switched namespace while a run was in flight. Its
    /// findings are about a deployment this window no longer shows —
    /// dropped, not displayed, and not an error either.
    #[test]
    fn a_run_from_another_namespace_is_not_evidence_here() {
        let mut state = DoctorState {
            deep: true,
            clocks_synced: true,
            ..DoctorState::default()
        };
        state.finish(
            run("acme", vec![finding(CheckId::TokenMissing, "a/b@00")]),
            "acme",
        );
        assert!(state.current.is_some());

        state.clear();
        assert!(state.deep && state.clocks_synced, "the toggles are kept");
        state.finish(
            run("acme", vec![finding(CheckId::StorageOnState, "s@z")]),
            "other",
        );
        assert!(
            state.current.is_none(),
            "a run about acme says nothing about other"
        );
        assert!(state.delta.is_none());
        assert!(state.error.is_none(), "and it is not an error either");
    }

    /// A zk2 key navigates to itself under the namespace, an address
    /// subject to the address (its instance id taken off), and a revision,
    /// a storage or the domain nowhere.
    #[test]
    fn finding_targets_resolve_by_subject_shape() {
        let nav: Addr = "robot1/nav".parse().unwrap();
        for subject in [
            "robot1/nav@0000000000000001",
            "robot1/nav input",
            "robot1/nav@0000000000000001 nav.v2",
            "robot1/nav nav.v2 plans/{vehicle}",
        ] {
            assert_eq!(
                finding_target(&finding(CheckId::TokenMissing, subject), "acme"),
                Some(Target::Address(nav.clone())),
                "{subject}"
            );
        }
        assert_eq!(
            finding_target(
                &finding(
                    CheckId::StateStampForeign,
                    "zk2/robot1/nav/nav.v2/state/status"
                ),
                "acme"
            ),
            Some(Target::Key(
                "acme/zk2/robot1/nav/nav.v2/state/status".into()
            ))
        );
        for subject in [
            "nav.v2 sha256:0123",
            "state@z1",
            "presence domain",
            "this host",
        ] {
            assert_eq!(
                finding_target(&finding(CheckId::StorageOnState, subject), ""),
                None,
                "{subject} navigates nowhere"
            );
        }
    }
}

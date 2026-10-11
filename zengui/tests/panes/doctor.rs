//! The doctor pane on zk2's doctor (#614, FL9): one verdict per check, the
//! four poles kept apart, and an empty scope never green.

use std::sync::Arc;

use zengui::doctor::{DoctorRun, DoctorState};
use zengui::view::doctor::section;
use zenkey_fleet::report::{
    Asked, CheckId, CheckReport, DoctorFinding, DoctorPresence, DoctorReport, DoctorScope,
    DoctorSeverity, Unjudged,
};

use super::*;

fn scope(presence: bool) -> DoctorScope {
    DoctorScope {
        namespace: "spray".into(),
        presence: if presence {
            Asked::Asked(DoctorPresence {
                selector: "zk2/*/*/@zk/**".into(),
                complete: true,
                grace_s: 2.0,
                services: 9,
                instances: 9,
                tokens: 21,
                undescribed: 0,
                revisions: 10,
                held: 10,
            })
        } else {
            Asked::NotAsked
        },
        routers: Asked::Asked(1),
        health: Asked::NotAsked,
    }
}

fn finding(
    check: CheckId,
    severity: DoctorSeverity,
    subject: &str,
    evidence: &str,
) -> DoctorFinding {
    DoctorFinding {
        severity,
        check,
        subject: subject.into(),
        evidence: evidence.into(),
    }
}

/// A report with one verdict per check: `findings` under theirs, the
/// `deep` checks not asked, `health-stale` unobservable with a subject left
/// undecided, and every other check clean.
fn report(findings: Vec<DoctorFinding>) -> DoctorReport {
    let checks = CheckId::ALL
        .into_iter()
        .map(|id| {
            if id.reads_data() {
                return CheckReport::not_asked(id);
            }
            let mine: Vec<DoctorFinding> =
                findings.iter().filter(|f| f.check == id).cloned().collect();
            let unjudged = if id == CheckId::HealthStale {
                vec![Unjudged {
                    subject: "host-a/sysmon".into(),
                    reason: "a status reply's age is unobservable without --clocks-synced".into(),
                }]
            } else {
                vec![]
            };
            CheckReport::of(id, mine, unjudged, "nothing fired")
        })
        .collect();
    DoctorReport {
        scope: scope(true),
        checks,
        unobservable: None,
    }
}

fn landed(report: DoctorReport) -> DoctorState {
    let mut state = DoctorState::default();
    state.finish(
        DoctorRun {
            report: Arc::new(report),
            namespace: "spray".into(),
        },
        "spray",
    );
    state
}

/// Never-run is not "0 findings"; a run renders one verdict per check in
/// the judgement shape — the finding, clean with its reason, unobservable
/// with its reason, not asked — each in its own words, and the findings
/// with their zk2 check ids and sections.
#[test]
fn the_doctor_pane_keeps_every_check_verdict_apart() {
    let state = DoctorState::default();
    let mut ui = simulator::<Message, _, _>(section(&state, "spray", sp()));
    assert!(
        ui.find("no doctor run yet").is_ok(),
        "never-run must not read as a clean deployment (O4)"
    );
    drop(ui);

    let state = landed(report(vec![finding(
        CheckId::TokenMissing,
        DoctorSeverity::Error,
        "robot9/phantom@00000000000000aa",
        "holds an interface token for nav.v2 and no instance token",
    )]));
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        (1600.0, 3000.0),
        section(&state, "spray", sp()),
    );
    assert!(
        ui.find("token-missing: finding").is_ok(),
        "the finding is the yes"
    );
    assert!(
        ui.find("split-brain: clean").is_ok(),
        "a clean check says clean"
    );
    assert!(ui.find("nothing fired").is_ok(), "with its reason");
    assert!(
        ui.find("health-stale: unobservable").is_ok(),
        "a subject left undecided is unobservable, never clean"
    );
    assert!(
        ui.find("host-a/sysmon: a status reply's age is unobservable without --clocks-synced")
            .is_ok()
    );
    assert!(
        ui.find("state-stamp-foreign: not asked").is_ok(),
        "a check the run was not to ask is not asked, never \"no\""
    );
    assert!(
        ui.find("token-missing").is_ok(),
        "the stable check id renders"
    );
    assert!(
        ui.find("robot9/phantom@00000000000000aa").is_ok(),
        "the subject, as the delta keys on it"
    );
    assert!(
        ui.find("go to subject").is_ok(),
        "an address subject offers navigation"
    );
    assert!(
        ui.find(
            "namespace \"spray\" · 9 services · 9 instances · 21 tokens · 10 of 10 revisions held \
             · 1 router"
        )
        .is_ok(),
        "the scope makes the silences legible"
    );
    assert!(
        ui.find("run: a finding at or above warning").is_ok(),
        "the run as one judgement"
    );
}

/// An empty scope — no zk2 token visible — is no verdict, however clean
/// every other check reads: a doctor pointed at the wrong namespace must not
/// be green.
#[test]
fn an_empty_scope_is_no_verdict() {
    let mut r = report(vec![]);
    let why = "no zk2 token visible to this reader in namespace \"spray\"";
    r.unobservable = Some(why.into());
    for c in r.checks.iter_mut().filter(|c| c.check.reads_presence()) {
        *c = CheckReport::unobservable(c.check, why);
    }
    let state = landed(r);
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        (1600.0, 3000.0),
        section(&state, "spray", sp()),
    );
    assert!(ui.find(format!("unobservable — {why}").as_str()).is_ok());
    assert!(
        ui.find(format!("run: no verdict — {why}").as_str()).is_ok(),
        "the run is no verdict"
    );
    assert!(
        ui.find("run: clean — no finding at or above warning among the 17 check(s) asked")
            .is_err(),
        "and never clean"
    );
}

/// A second run is a delta against the first, keyed on `(check, subject)`.
#[test]
fn a_rerun_shows_the_delta() {
    let mut state = landed(report(vec![finding(
        CheckId::TokenMissing,
        DoctorSeverity::Error,
        "robot9/phantom@00000000000000aa",
        "e",
    )]));
    state.finish(
        DoctorRun {
            report: Arc::new(report(vec![finding(
                CheckId::StorageOnState,
                DoctorSeverity::Error,
                "state@z1",
                "captures `**`",
            )])),
            namespace: "spray".into(),
        },
        "spray",
    );
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        (1600.0, 3000.0),
        section(&state, "spray", sp()),
    );
    assert!(
        ui.find("vs previous run: 1 new · 1 fixed · 0 unchanged")
            .is_ok(),
        "the delta strip renders"
    );
    assert!(
        ui.find("Fixed since last run").is_ok(),
        "fixed findings stay visible, dimmed"
    );
    assert!(ui.find("new").is_ok(), "the new finding is marked new");
}

/// v1's "re-ask schemas" is zk2's "re-read presence and contracts", and it
/// says which state it is in — a button whose effect you cannot observe is
/// one you cannot trust (#101).
#[test]
fn the_doctor_pane_offers_the_reread_and_reports_it() {
    let mut state = DoctorState::default();
    {
        let mut ui = simulator::<Message, _, _>(section(&state, "", sp()));
        assert!(ui.find("re-read presence and contracts").is_ok());
        assert!(ui.find("re-ask schemas").is_err(), "v1's words are gone");
    }
    state.rereads = 2;
    let mut ui = simulator::<Message, _, _>(section(&state, "", sp()));
    assert!(
        ui.find(
            "presence and the contracts re-read 2 times — every revision no holder served \
             is asked again"
        )
        .is_ok()
    );
}

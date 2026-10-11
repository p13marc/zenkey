//! The doctor pane.

use super::*;

/// The doctor panel (#71): never-run is not "0 findings", findings render
/// with their check ids and RFC citations, and re-runs show deltas — all
/// over the exact struct `zenctl doctor --format json` serializes.
#[test]
fn the_doctor_pane_never_invents_a_verdict() {
    use std::sync::Arc;
    use zengui::doctor::DoctorState;
    use zengui::view::doctor::section;
    use zenkey_fleet_v1::report::{DoctorFinding, DoctorReport, DoctorSeverity};

    let state = DoctorState::default();
    let mut ui = simulator::<Message, _, _>(section(&state, "", sp()));
    assert!(
        ui.find("no doctor run yet").is_ok(),
        "never-run must not read as a clean fleet (O4)"
    );

    let report = DoctorReport {
        findings: vec![DoctorFinding {
            severity: DoctorSeverity::Error,
            check: zenkey_fleet_v1::report::CheckId::SliceSync,
            subject: "h-3fa9c2d41b7e/sysinfo".into(),
            evidence: "registry version differs: served 1.0, local 2.0".into(),
            citation: Some("RFC 08 §6".into()),
        }],
        synced: zenkey_fleet_v1::report::Asked::NotAsked,
        introspect_answered: 1,
        live_producers: 1,
        describe_served: 0,
        describe_missing: 1,
        routers: 0,
        router_version: None,
        deep: false,
        observation: None,
        unobservable: None,
    };
    let mut state = DoctorState::default();
    state.finish(
        Ok(zengui::doctor::DoctorRun {
            report: Arc::new(report.clone()),
            base: String::new(),
        }),
        "",
    );
    let mut ui = simulator::<Message, _, _>(section(&state, "", sp()));
    assert!(ui.find("slice-sync").is_ok(), "the stable check id renders");
    assert!(ui.find("RFC 08 §6").is_ok(), "the citation renders");
    assert!(
        ui.find("registry version differs: served 1.0, local 2.0")
            .is_ok(),
        "the evidence renders"
    );
    assert!(
        ui.find("go to subject").is_ok(),
        "an origin/producer subject offers navigation"
    );

    drop(ui);

    // Second run: the finding is fixed, a new one appears — the delta strip
    // says exactly that.
    let second = DoctorReport {
        findings: vec![DoctorFinding {
            severity: DoctorSeverity::Info,
            check: zenkey_fleet_v1::report::CheckId::DescribeMissing,
            subject: "fleet".into(),
            evidence: "1 producer(s) serve no describe".into(),
            citation: Some("RFC 08 §7".into()),
        }],
        ..report
    };
    state.finish(
        Ok(zengui::doctor::DoctorRun {
            report: Arc::new(second),
            base: String::new(),
        }),
        "",
    );
    let mut ui = simulator::<Message, _, _>(section(&state, "", sp()));
    assert!(
        ui.find("vs previous run: 1 new · 1 fixed · 0 unchanged")
            .is_ok(),
        "the delta strip renders"
    );
    assert!(
        ui.find("Fixed since last run").is_ok(),
        "fixed findings stay visible, dimmed"
    );
}

/// #161: the listen phase's observation section renders its scope statement
/// — window, scopes, drops, synthetic count — and a report without one shows
/// nothing (absent, never zeroed).
#[test]
fn the_doctor_pane_states_what_the_listen_phase_observed() {
    use std::sync::Arc;
    use zengui::doctor::DoctorState;
    use zengui::view::doctor::section;
    use zenkey_fleet_v1::report::{DoctorReport, ObservationSummary};

    let report = DoctorReport {
        findings: vec![],
        synced: zenkey_fleet_v1::report::Asked::NotAsked,
        introspect_answered: 1,
        live_producers: 1,
        describe_served: 0,
        describe_missing: 1,
        routers: 0,
        router_version: None,
        deep: false,
        observation: Some(ObservationSummary {
            window_s: 10.0,
            scopes: vec!["v1/*/state/**".into(), "v1/*/telemetry/**".into()],
            samples: 42,
            keys_seen: 7,
            dropped: 3,
            synthetic_marked: 2,
            field_paths_dropped: 0,
            facts_evicted: 0,
        }),
        unobservable: None,
    };
    let mut state = DoctorState::default();
    state.finish(
        Ok(zengui::doctor::DoctorRun {
            report: Arc::new(report),
            base: String::new(),
        }),
        "",
    );
    let mut ui = simulator::<Message, _, _>(section(&state, "", sp()));
    assert!(
        ui.find(
            "listened 10s over 2 scopes: 42 samples on 7 keys, 3 dropped \
                 (findings cover only what was seen — O6) · 2 synthetic-marked"
        )
        .is_ok(),
        "the observation section states window, scope, drops, and synthetic count"
    );
}

/// The schema cache's escape hatch (#101) is visible, and says which state it
/// is in — a button whose effect you cannot observe is one you cannot trust.
#[test]
fn the_doctor_pane_offers_the_schema_re_ask_and_reports_it() {
    use zengui::doctor::DoctorState;
    use zengui::view::doctor::section;

    let mut state = DoctorState::default();
    {
        let mut ui = simulator::<Message, _, _>(section(&state, "", sp()));
        assert!(ui.find("re-ask schemas").is_ok());
        assert!(
            ui.find(
                "schema cache: kept for the session; a producer that changes its \
                 served set is read with the schemas it had at first contact"
            )
            .is_ok(),
            "the standing behaviour is stated, not left to be discovered"
        );
    }

    state.schemas_forgotten = 1;
    {
        let mut ui = simulator::<Message, _, _>(section(&state, "", sp()));
        assert!(
            ui.find("schema cache cleared 1 time — the next decode asks the bus again")
                .is_ok()
        );
    }
    state.schemas_forgotten = 3;
    {
        let mut ui = simulator::<Message, _, _>(section(&state, "", sp()));
        assert!(
            ui.find("schema cache cleared 3 times — the next decode asks the bus again")
                .is_ok()
        );
    }
}

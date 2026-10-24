//! The Inspector's fields, why and consumers sections — zk2's since #614
//! (FL8b).

use super::*;

/// The section, rendered on its own.
fn draw(col: iced::widget::Column<'_, Message>) -> iced_test::Simulator<'_, Message> {
    simulator::<Message, _, _>(iced::Element::from(iced::widget::container(col)))
}

/// #223 on zk2: the Fields section states its window, its coverage, every
/// bound's cost and what it decoded with — and never-run is not "no
/// fields", nor a key no contract resolved "clean" (O4).
#[test]
fn the_fields_section_states_its_window_and_its_bounds() {
    use std::sync::Arc;
    use zengui::view::fields::{FieldsState, section};
    use zenkey_fleet::report::{FieldReport, FieldRow, LensScope};

    // Never run: the section explains its cost instead of claiming absence.
    let state = FieldsState::default();
    let mut ui = draw(section(&state, zengui::message::SlotId::FOLLOW, sp()));
    assert!(
        ui.find(
            "no field observation yet — the button subscribes to exactly this \
             key for the window, then releases; nothing ambient"
        )
        .is_ok(),
        "never-run states the cost, not an empty table"
    );

    // A landed report: coverage, the path bound's refusals, the lens it
    // decoded through, and what it could not judge.
    let key = common::ETH0;
    let report = Some(Ok(Arc::new(FieldReport {
        selector: key.into(),
        window_s: 10.0,
        lens: LensScope {
            namespace: String::new(),
            presence: None,
            contracts: 1,
        },
        samples: 40,
        keys_seen: 1,
        dropped: 2,
        undocumented: 3,
        unread: 0,
        unresolved: 5,
        paths: 512,
        max_paths: 512,
        paths_dropped: 7,
        paths_dropped_examples: vec![format!("{key} · f99")],
        rows: vec![FieldRow {
            key: key.into(),
            path: "link_speed_mbps".into(),
            declared: Some(true),
            seen: 40,
            documents: 40,
            kinds: vec!["number".into()],
            changes: 0,
            last_change_s: None,
            min: Some(1000.0),
            max: Some(1000.0),
            last: Some(1000.0),
            values: None,
        }],
        findings: vec![],
    })));
    let state = FieldsState {
        report,
        ..FieldsState::default()
    };
    let mut ui = draw(section(&state, zengui::message::SlotId::FOLLOW, sp()));
    assert!(
        ui.find(format!(
            "watched {key} for 10s: 40 samples on 1 key · 2 dropped · 3 without a document"
        ))
        .is_ok(),
        "the window and its coverage are stated (O5/O6)"
    );
    assert!(
        ui.find(format!(
            "512 paths tracked (bound 512) · 7 refused for the bound — e.g. {key} · f99"
        ))
        .is_ok(),
        "the path table's bound reports and names what it refused"
    );
    assert!(
        ui.find("decoded through the lens: namespace (the bus root) · no presence read · 1 revision held")
            .is_ok(),
        "what the payloads were decoded with"
    );
    assert!(
        ui.find(
            "5 samples on keys no contract resolved — observed structurally, and field-new \
             unjudgeable for them (O4), not clean"
        )
        .is_ok(),
        "an unresolved key is unjudgeable, never clean"
    );
    assert!(
        ui.find("field-stuck is not asked — it judges against a declared freshness, freshness.v1's (#613)")
            .is_ok()
    );
    assert!(
        ui.find(
            "link_speed_mbps · seen 40/40 · number · declared · unchanged in the window \
             · min 1000 · max 1000 · last 1000"
        )
        .is_ok(),
        "an unchanged path is scoped to the window, not 'never'"
    );
}

/// #214 on zk2 (#702): the Why section renders every rung of the ladder,
/// and a rung the ladder never reached reads "not asked" — never "no", and
/// never "unobservable"; a cause is named at its rung.
#[test]
fn the_why_section_renders_not_asked_and_never_no() {
    use std::sync::Arc;
    use zengui::view::why::{WhyState, section};
    use zenkey_fleet::report::Judgement;
    use zenkey_fleet::{WhyObservation, WhySpec, WhyTarget};

    let spec = WhySpec {
        timeout: std::time::Duration::from_secs(1),
        window: std::time::Duration::from_secs(3),
    };
    // Never run: the section states what a run reads.
    let state = WhyState::default();
    let mut ui = draw(section(&state, zengui::message::SlotId::FOLLOW, sp()));
    assert!(ui.find("why?").is_ok(), "the one button");
    assert!(ui.find("not asked yet — \"why?\" runs zk2's silence ladder here: presence, the descriptor, the contract, then the answer as the key's kind is read (S4's GET, a short listen, a bounded GET) and, after an owner's silence, the archives — nothing is called").is_ok());

    // A ladder over nothing read: past the rungs the key alone decides,
    // every rung is not asked, and says so.
    let report = zenkey_fleet::judge_why(&WhyObservation::new(
        "",
        WhyTarget::Key(common::NAV_STATUS.into()),
        spec,
    ));
    assert!(
        report
            .rungs
            .iter()
            .any(|r| matches!(r.verdict, Judgement::NotAsked)),
        "setup: some rung must be not asked"
    );
    let state = WhyState {
        report: Some(Ok(Arc::new(report.clone()))),
        ..WhyState::default()
    };
    let mut ui = draw(section(&state, zengui::message::SlotId::FOLLOW, sp()));
    assert!(
        ui.find("not asked").is_ok(),
        "an unreached rung renders 'not asked', never 'no'"
    );
    for rung in &report.rungs {
        assert!(
            ui.find(format!("{} ({})", rung.rung, rung.section)).is_ok(),
            "every rung renders, the unasked ones included: {}",
            rung.rung
        );
    }
    assert!(ui.find("no").is_err(), "no rung answers a bare 'no'");

    // A key outside the namespace: a cause, from the key alone, at its rung.
    let report = zenkey_fleet::judge_why(&WhyObservation::new(
        "spray",
        WhyTarget::Key("elsewhere/zk2/robot1/nav/nav.v2/state/status".into()),
        spec,
    ));
    let (rung, cause) = report.cause().expect("a cause");
    let headline = format!("why {} — a cause, at {rung}: {cause}", report.target);
    let state = WhyState {
        report: Some(Ok(Arc::new(report.clone()))),
        ..WhyState::default()
    };
    let mut ui = draw(section(&state, zengui::message::SlotId::FOLLOW, sp()));
    assert!(ui.find(headline).is_ok(), "the cause, named at its rung");
    assert!(ui.find("cause").is_ok());
    assert!(
        ui.find("asked the bus nothing — the key alone decided where the ladder stopped")
            .is_ok(),
        "what was put to the bus: nothing (O5)"
    );
}

/// The Consumers section (#224; R3 since #614, FL8b) never draws matching
/// status (D13): never-run reads "not asked", no presence read reads "not
/// asked, never none", a landed answer says it is R3 from descriptors and
/// names each role bound — and none of it says "listening", "matching",
/// "unmatched" or "no consumers".
#[test]
fn the_consumers_section_says_r3_from_descriptors_and_never_matching_status() {
    use std::sync::Arc;
    use zengui::view::consumers::{
        Consumers, ConsumersAnswer, ConsumersState, ConsumersTarget, section,
    };
    use zenkey_fleet::report::{BindingGraph, GraphEdge};

    const FORBIDDEN: &[&str] = &["matching", "listening", "unmatched", "no consumers"];
    let find_none = |ui: &mut iced_test::Simulator<'_, Message>| {
        for w in FORBIDDEN {
            assert!(
                ui.find(*w).is_err(),
                "{w:?} is matching-status vocabulary (D13)"
            );
        }
    };
    let state = |answer: ConsumersAnswer| ConsumersState {
        report: Some(Arc::new(answer)),
    };

    // Never run: not asked, and what a press reads.
    let never = ConsumersState::default();
    let mut ui = draw(section(&never, zengui::message::SlotId::FOLLOW, sp()));
    assert!(ui.find("consumers?").is_ok(), "the one button");
    assert!(
        ui.find(
            "not asked yet — \"consumers?\" reads the roles the descriptors bind to this \
             subject (R3, from the presence read in hand); a binding is never proof that \
             anything reads it (D13)"
        )
        .is_ok()
    );
    find_none(&mut ui);

    // No presence read: not asked, never an empty set.
    let s = state(ConsumersAnswer::PresenceNotRead);
    let mut ui = draw(section(&s, zengui::message::SlotId::FOLLOW, sp()));
    assert!(ui.find("Presence not read").is_ok());
    find_none(&mut ui);

    // A landed answer: R3, from descriptors; each role a card.
    let graph = BindingGraph {
        complete: true,
        cut_off_during: false,
        nodes: Vec::new(),
        edges: vec![
            GraphEdge {
                consumer: "fleet/planner".into(),
                role: "nav".into(),
                interface: "nav.v2".into(),
                provider: "robot1/nav".into(),
            },
            GraphEdge {
                consumer: "fleet/planner".into(),
                role: "nav".into(),
                interface: "nav.v2".into(),
                provider: "robot2/nav".into(),
            },
        ],
        undescribed: Vec::new(),
    };
    let target = ConsumersTarget {
        address: "robot1/nav".into(),
        iface: Some("nav.v2".into()),
    };
    let s = state(ConsumersAnswer::Read(Consumers::of(
        &graph,
        target.clone(),
        true,
    )));
    let mut ui = draw(section(&s, zengui::message::SlotId::FOLLOW, sp()));
    assert!(
        ui.find(
            "R3: from descriptors — 1 role bound to robot1/nav · nav.v2; a binding names a \
             provider of the interface, never a claim that anything reads, nor of what flows \
             (D13)"
        )
        .is_ok(),
        "only the edge whose provider is the subject's address"
    );
    assert!(ui.find("fleet/planner").is_ok());
    assert!(ui.find("binds robot1/nav for nav.v2").is_ok());
    assert!(ui.find("binds robot2/nav for nav.v2").is_err());
    find_none(&mut ui);

    // Nothing bound: a reading of what was declared, not a verdict.
    let s = state(ConsumersAnswer::Read(Consumers::of(
        &graph,
        ConsumersTarget {
            address: "host-a/tc".into(),
            iface: None,
        },
        false,
    )));
    let mut ui = draw(section(&s, zengui::message::SlotId::FOLLOW, sp()));
    assert!(ui.find("No role binds this subject").is_ok());
    find_none(&mut ui);
}

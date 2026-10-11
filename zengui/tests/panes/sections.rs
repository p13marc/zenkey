//! The Inspector's fields, why and consumers sections.

use super::*;

/// #223: the Fields section states its window, its coverage, and every
/// bound's cost — and never-run is not "no fields" (O4).
#[test]
fn the_fields_section_states_its_window_and_its_bounds() {
    use std::sync::Arc;
    use zengui::view::fields::{FieldsState, section};
    use zenkey_fleet_v1::report::{FieldReport, FieldRow};

    // Never run: the section explains its cost instead of claiming absence.
    let state = FieldsState::default();
    let mut ui = simulator::<Message, _, _>(iced::Element::from(iced::widget::container(section(
        &state,
        zengui::message::SlotId::FOLLOW,
        sp(),
    ))));
    assert!(
        ui.find(
            "no field observation yet — the button subscribes to exactly this \
             key for the window, then releases; nothing ambient"
        )
        .is_ok(),
        "never-run states the cost, not an empty table"
    );

    // A landed report: coverage, the path bound's refusals, and the
    // no-registry caveat all render.
    let report = Some(Ok(Arc::new(FieldReport {
        selector: "v1/h-a/telemetry/sysinfo/cpu".into(),
        window_s: 10.0,
        samples: 40,
        keys_seen: 1,
        dropped: 2,
        undocumented: 3,
        unread: 0,
        registry_loaded: false,
        paths: 512,
        max_paths: 512,
        paths_dropped: 7,
        paths_dropped_examples: vec!["v1/h-a/telemetry/sysinfo/cpu · f99".into()],
        facts_evicted: 0,
        rows: vec![FieldRow {
            key: "v1/h-a/telemetry/sysinfo/cpu".into(),
            path: "temperature_c".into(),
            seen: 40,
            documents: 40,
            kinds: vec!["number".into()],
            changes: 0,
            last_change_s: None,
            min: Some(21.5),
            max: Some(21.5),
            last: Some(21.5),
            values: None,
        }],
        findings: vec![],
    })));
    let state = FieldsState {
        report,
        ..FieldsState::default()
    };
    let mut ui = simulator::<Message, _, _>(iced::Element::from(iced::widget::container(section(
        &state,
        zengui::message::SlotId::FOLLOW,
        sp(),
    ))));
    assert!(
        ui.find(
            "watched v1/h-a/telemetry/sysinfo/cpu for 10s: 40 samples on \
             1 key · 2 dropped · 3 without a structural document"
        )
        .is_ok(),
        "the window and its coverage are stated (O5/O6)"
    );
    assert!(
        ui.find(
            "512 paths tracked (bound 512) · 7 refused for the bound — \
             e.g. v1/h-a/telemetry/sysinfo/cpu · f99"
        )
        .is_ok(),
        "the path table's bound reports and names what it refused"
    );
    assert!(
        ui.find(
            "no registry loaded — declared ttl_s and types are unknown, so \
             field-stuck and field-new are unjudgeable here (O4), not clean"
        )
        .is_ok(),
        "no registry reads as unjudgeable, never as clean"
    );
    assert!(
        ui.find(
            "temperature_c · seen 40/40 · number · unchanged in the window \
             · min 21.5 · max 21.5 · last 21.5"
        )
        .is_ok(),
        "an unchanged path is scoped to the window, not 'never'"
    );
}

/// #214: the Why section renders every rung, and a rung whose input was not
/// fetched reads "not asked" — never "no". The impaired verdict refuses to
/// claim health over questions it could not ask.
#[test]
fn the_why_section_renders_not_asked_and_never_no() {
    use std::sync::Arc;
    use zengui::view::why::{WhyState, section};
    use zenkey_fleet_v1::judge::why::{WhyInputs, ladder};
    use zenkey_fleet_v1::report::RungAnswer;

    // Never run: the section states the frugal default's cost.
    let state = WhyState::default();
    let mut ui = simulator::<Message, _, _>(iced::Element::from(iced::widget::container(section(
        &state,
        zengui::message::SlotId::FOLLOW,
        sp(),
    ))));
    assert!(
        ui.find(
            "not asked yet — \"why?\" runs the silence ladder on this key: \
             control-plane sweeps and one bounded GET, no subscriber \
             (RFC v1.18 frugality)"
        )
        .is_ok(),
    );

    // A ladder over nothing fetched: rungs degrade to NotAsked, and the
    // report is impaired rather than healthy.
    let report = ladder(&WhyInputs {
        base: "",
        key: "v1/h-3fa9c2d41b7e/state/sysinfo/health",
        slices: None,
        roster: None,
        entities: None,
        admin_answered: None,
        storages: None,
        stored: None,
        wire: None,
    });
    assert!(
        report
            .rungs
            .iter()
            .any(|r| matches!(r.answer, RungAnswer::NotAsked)),
        "setup: some rung must be NotAsked"
    );
    let state = WhyState {
        report: Some(Ok(Arc::new(report))),
        ..WhyState::default()
    };
    let mut ui = simulator::<Message, _, _>(iced::Element::from(iced::widget::container(section(
        &state,
        zengui::message::SlotId::FOLLOW,
        sp(),
    ))));
    assert!(
        ui.find("not asked").is_ok(),
        "an unfetched input renders 'not asked', never 'no'"
    );
    assert!(
        ui.find("wire-heard").is_ok(),
        "every rung renders, the unasked ones included"
    );
    assert!(
        ui.find(
            "no cause established, and the observation was impaired — \
             \"healthy\" cannot be claimed over questions that could not be \
             asked"
        )
        .is_ok(),
        "impaired refuses to over-claim"
    );
}

/// The Consumers section (#224) never draws matching status (RFC 12 §9):
/// never-run reads "not asked", an admin space that did not answer reads
/// "not asked, never none", a landed join names the tool's own session and
/// flags the whole-base wildcard — and none of it says "listening",
/// "matching", "unmatched" or "no consumers".
#[test]
fn the_consumers_section_renders_not_asked_and_never_matching_status() {
    use std::sync::Arc;
    use zengui::view::consumers::{ConsumersState, section};
    use zenkey_fleet_v1::report::{
        AdminAnswer, Attribution, ConsumerRow, ConsumersReport, EntityKind, Relation,
    };

    const FORBIDDEN: &[&str] = &["matching", "listening", "unmatched", "no consumers"];
    let find_none = |ui: &mut iced_test::Simulator<'_, Message>, words: &[&str]| {
        for w in words {
            assert!(
                ui.find(*w).is_err(),
                "{w:?} is matching-status vocabulary (RFC 12 §9)"
            );
        }
    };

    // Never run: not asked, and the cost stated.
    let state = ConsumersState::default();
    let mut ui = simulator::<Message, _, _>(iced::Element::from(iced::widget::container(section(
        &state,
        zengui::message::SlotId::FOLLOW,
        sp(),
    ))));
    assert!(ui.find("consumers?").is_ok(), "the one button");
    assert!(
        ui.find(
            "not asked yet — \"consumers?\" sweeps the admin space once for every \
             declared subscriber and querier related to this subject; a \
             declaration is not proof of use (RFC 12 §9)"
        )
        .is_ok()
    );
    find_none(&mut ui, FORBIDDEN);

    // No admin space answered: not asked, never an empty set.
    let report = ConsumersReport {
        target: "v1/h-3fa9c2d41b7e/state/sysinfo/health".into(),
        asked: vec!["@/*/*".into()],
        self_zid: "ffffffff".into(),
        admin: AdminAnswer::NotAvailable,
        rows: vec![],
        reply_elided: 0,
    };
    let state = ConsumersState {
        report: Some(Ok(Arc::new(report.clone()))),
        ..ConsumersState::default()
    };
    let mut ui = simulator::<Message, _, _>(iced::Element::from(iced::widget::container(section(
        &state,
        zengui::message::SlotId::FOLLOW,
        sp(),
    ))));
    assert!(
        ui.find(
            "no admin space answered — zenoh's adminspace.enabled is off by \
             default; the declared readers are not asked, never none (RFC 13 §3 O4)"
        )
        .is_ok()
    );
    find_none(&mut ui, FORBIDDEN);

    // A landed join: the self row named, the wildcard flagged as total.
    let row = ConsumerRow {
        zid: "ffffffff".into(),
        whatami: Some("peer".into()),
        origins: vec![],
        attribution: Attribution::Session,
        kind: EntityKind::Subscriber,
        keyexpr: "**".into(),
        relation: Relation::Total,
        is_self: true,
        total_wildcard: true,
    };
    let state = ConsumersState {
        report: Some(Ok(Arc::new(ConsumersReport {
            admin: AdminAnswer::Answered {
                answered: 1,
                nodes: 2,
            },
            rows: vec![row],
            ..report
        }))),
        ..ConsumersState::default()
    };
    let mut ui = simulator::<Message, _, _>(iced::Element::from(iced::widget::container(section(
        &state,
        zengui::message::SlotId::FOLLOW,
        sp(),
    ))));
    assert!(ui.find("ffffffff  (this zengui session) · peer").is_ok());
    assert!(ui.find("session only, unattributed").is_ok());
    assert!(
        // Its leading spaces went with #564: the card groups it now.
        ui.find("subscriber ** — total — a whole-base `**`, intersects everything")
            .is_ok()
    );
    find_none(&mut ui, FORBIDDEN);
}

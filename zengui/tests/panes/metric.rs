//! `telemetry.v1` in the Inspector (#819, TD): a metric's unit read from its
//! declaration — zenoh-modem's among them — its value plotted, and the
//! Fields sparks in its unit.

use super::*;

use zengui::metric::Unit;
use zenkey_model::telemetry::Source;

/// The metered world is honest: every sample it publishes conforms to its
/// declaration through the lens the window resolves with — so a scene's
/// one non-conforming Point is the only one.
#[test]
fn the_metered_world_conforms_to_its_declarations() {
    let mut world = common::World::metered();
    let mut app = common::window(zengui::prefs::Prefs::default());
    world.connect(&mut app);
    world.ticks(&mut app, 30);
    let metered: Vec<&str> = common::World::metered_keys().collect();
    let mut checked = 0;
    for (key, bytes, encoding, attachment) in world.published() {
        if !metered.contains(&key) {
            continue;
        }
        let c = world.check_attached(key, bytes, encoding, attachment);
        assert!(c.identity.unresolved.is_none(), "{key}: {:?}", c.identity);
        assert_eq!(
            c.conformance,
            zenkey_fleet::report::Conformance::Valid,
            "{key}"
        );
        checked += 1;
    }
    assert_eq!(checked, metered.len());
}

/// zenoh-modem's units, read from its declarations (#819's "done when"):
/// every stream and state of `modem.v3` that writes `telemetry.unit` reads
/// that unit, declared, through the lens and `zengui::metric` — and a
/// metric that writes none reads unknown, never dimensionless.
#[test]
fn modems_units_are_read_from_its_declarations() {
    let world = common::World::metered();
    let loaded = world.loaded();
    let iface: zenkey_model::grammar::IfaceId = "modem.v3".parse().unwrap();
    let revision = loaded.set.of_iface(&iface).next().expect("modem.v3 held");
    let ring = zengui::history::HistoryRing::new(4);
    let (mut units, mut unknown) = (0, 0);
    for r in &revision.contract().resources {
        if !matches!(
            r.kind,
            zenkey_model::authoring::Kind::Stream | zenkey_model::authoring::Kind::State
        ) || r.template.as_str().contains('{')
        {
            continue;
        }
        let key = format!(
            "zk2/{}/modem.v3/{}/{}",
            common::MODEM,
            r.token.as_str(),
            r.template.as_str()
        );
        let resolution = world.resolve(&key);
        let resolved = resolution
            .resolved
            .as_ref()
            .unwrap_or_else(|| panic!("{key}"));
        let Some(m) = zengui::metric::read(resolved, &ring) else {
            continue;
        };
        match r
            .annotations
            .get(zenkey_model::telemetry::UNIT)
            .and_then(|u| u.as_str())
        {
            Some(u) => {
                assert_eq!(
                    m.unit(),
                    Unit::Known(u.to_string(), Source::Declared),
                    "{key}"
                );
                units += 1;
            }
            None => {
                assert_eq!(m.unit(), Unit::Unknown, "{key}");
                assert_eq!(m.unit().words(), "unknown (§2.5)");
                unknown += 1;
            }
        }
    }
    assert_eq!(
        units, 23,
        "telemetry.v1 Appendix B: modem declares 23 units"
    );
    assert!(unknown > 0, "and many of its counters declare none");
}

/// The Fields sparks carry a metric's unit on its value path alone: a
/// Point's `value.value` in the member's unit, its `timestamp` in none of
/// the member's (#819, TD).
#[test]
fn a_fields_spark_carries_the_unit_of_a_metrics_value() {
    use std::sync::Arc;
    use zengui::view::fields::{FieldsState, section, sparks_for};
    use zenkey_fleet::report::{FieldReport, FieldRow, LensScope};

    let world = common::World::metered();
    let mut rec = HistoryRecorder::new(common::FRAMES, 16);
    let t0 = Instant::now();
    for n in 0..6u32 {
        rec.observe(&common::sample_attached(
            common::FRAMES,
            &common::metric_point(
                n,
                "counter",
                serde_json::json!(10 * n),
                Some("1"),
                Some("00000000000000a1"),
            ),
            "application/json",
            t0 + std::time::Duration::from_millis(250 * u64::from(n)),
            common::Axes::DEFAULT,
            None,
        ));
    }
    let resolution = world.resolve(common::FRAMES);
    let metric =
        zengui::metric::read(resolution.resolved.as_ref().unwrap(), &rec.ring).expect("a metric");
    let row = |path: &str| FieldRow {
        key: common::FRAMES.into(),
        path: path.into(),
        declared: Some(true),
        seen: 6,
        documents: 6,
        kinds: vec!["number".into()],
        changes: 5,
        last_change_s: Some(1.25),
        min: Some(0.0),
        max: Some(50.0),
        last: Some(50.0),
        values: None,
    };
    let report = FieldReport {
        selector: common::FRAMES.into(),
        window_s: 2.0,
        lens: LensScope {
            namespace: String::new(),
            presence: None,
            contracts: 1,
        },
        samples: 6,
        keys_seen: 1,
        dropped: 0,
        undocumented: 0,
        unread: 0,
        unresolved: 0,
        paths: 2,
        max_paths: 512,
        paths_dropped: 0,
        paths_dropped_examples: vec![],
        rows: vec![row("value.value"), row("timestamp")],
        findings: vec![],
    };
    let sparks = sparks_for(&report, Some(&rec), Some(&metric));
    let unit_of = |p: &str| {
        sparks
            .iter()
            .find(|s| s.path == p)
            .map(|s| s.unit.clone())
            .unwrap_or_else(|| panic!("a spark for {p}"))
    };
    assert_eq!(
        unit_of("value.value"),
        Some(Unit::Known("1".into(), Source::Declared))
    );
    assert_eq!(unit_of("timestamp"), None, "no unit of the member's");
    let value = sparks.iter().find(|s| s.path == "value.value").unwrap();
    let caption = zengui::view::spark::caption_text(
        "value.value",
        &value.series,
        zengui::view::theme::SeriesTone::Value,
        Some("1"),
    );
    assert!(caption.starts_with("value.value [1]: 50 now"), "{caption}");
    let state = FieldsState {
        report: Some(Ok(Arc::new(report))),
        sparks,
        ..FieldsState::default()
    };
    let mut ui = simulator::<Message, _, _>(iced::Element::from(iced::widget::container(section(
        &state,
        zengui::message::SlotId::FOLLOW,
        sp(),
    ))));
    assert!(ui.find(caption.as_str()).is_ok(), "the spark says its unit");
}

/// The Inspector of the metered scenes (#819, TD): the counter's kind and
/// unit beside its type, its rate's time base and its reset's gap; a
/// counter with no unit said unknown, and no decrease claimed where no
/// epoch could find one.
#[test]
fn the_inspector_reads_a_counter_by_telemetry_v1() {
    let scene = |name: &str| {
        let s = common::scenes::ALL
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("scene {name}"));
        (s.build)(zengui::prefs::ThemeChoice::Dark)
    };
    let app = scene("inspector-metric-counter");
    let mut ui = simulator::<Message, _, _>(app.view(iced::window::Id::unique()));
    for text in [
        "counter · declared",
        "1 · declared",
        "telemetry.v1.Point",
        "read by telemetry.v1 over the recorded history: the Point's value.value (§2.5, §2.6)",
        "timed by the Points' timestamps, the measurement times (§2.11)",
        "no rate across a reset or a decrease: 1 pair drawn as gaps, never restarted from \
         zero (§2.11)",
    ] {
        assert!(ui.find(text).is_ok(), "{text}");
    }
    let app = scene("inspector-metric-unit-unknown");
    let mut ui = simulator::<Message, _, _>(app.view(iced::window::Id::unique()));
    for text in [
        "unknown (§2.5)",
        "unit unknown — no telemetry.unit is declared and no Point carried one \
         (telemetry.v1 §2.5): never read as dimensionless",
        "the counter's unit is unknown, so its rate's is too (§2.11)",
        "no decrease could be found: every pair's epoch was assumed — no in-band epoch \
         rides these samples — so a decrease reads as a reset, never a finding (§2.10, §5)",
        "1 reset: 0 in-band (another epoch), 1 assumed (a decrease no epoch places)",
    ] {
        assert!(ui.find(text).is_ok(), "{text}");
    }
    assert!(ui.find("decreased").is_err(), "no finding without an epoch");
}

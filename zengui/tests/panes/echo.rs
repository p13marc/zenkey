//! The echo rows' verdict badges.

use super::*;

/// #164: echo rows badge the *cached* verdict — a lookup, never a decode —
/// and a key never checked renders the third state in words: "not validated
/// (not yet checked)", never a blank that could read as fine and never a
/// borrowed verdict.
#[test]
fn the_echo_rows_badge_cached_verdicts_and_admit_the_unchecked() {
    use std::time::Instant;
    use zengui::echo::EchoRing;
    use zengui::verdict::VerdictCache;
    use zengui::view::echo::{EchoView, section};
    use zenkey_fleet_v1::SampleView;
    use zenkey_v1::schema::validate::Verdict;

    let sample = |key: &str| SampleView {
        key: key.to_string(),
        payload: zenoh::bytes::ZBytes::from(br#"{"v":1}"#.to_vec()),
        encoding: "application/json".into(),
        kind: zenoh::sample::SampleKind::Put,
        timestamp: None,
        stamped_by: None,
        attachment: None,
        priority: zenoh::qos::Priority::Data,
        congestion_control: zenoh::qos::CongestionControl::Drop,
        reliability: zenoh::qos::Reliability::BestEffort,
        express: false,
        source: None,
        received: Instant::now(),
    };
    let mut ring = EchoRing::new(10);
    ring.push(&sample("v1/h-3fa9c2d41b7e/state/sysinfo/health"));
    ring.push(&sample("v1/h-3fa9c2d41b7e/state/sysinfo/unchecked"));

    let mut verdicts = VerdictCache::default();
    verdicts.record(
        "v1/h-3fa9c2d41b7e/state/sysinfo/health",
        Verdict::Invalid(vec!["/v: wrong type".into()]),
    );

    let view = EchoView::new();
    let mut ui = simulator::<Message, _, _>(iced::Element::from(iced::widget::container(section(
        &ring,
        &view,
        None,
        ring.next_seq(),
        Default::default(),
        &verdicts,
        zengui::prefs::ThemeChoice::Dark,
        sp(),
    ))));
    assert!(
        ui.find("invalid (1 violation)").is_ok(),
        "the checked key wears its cached verdict"
    );
    assert!(
        ui.find("not validated (not yet checked)").is_ok(),
        "an unchecked key says so — the third state, never a blank"
    );
    assert!(
        ui.find("verdicts: 1 key checked (each badge is the key's most recently checked sample)")
            .is_ok(),
        "the strip scopes what a badge claims"
    );
}

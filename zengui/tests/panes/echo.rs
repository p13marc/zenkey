//! The echo rows' verdict badges.

use super::*;

/// #164, on zk2's four poles (#614, FL6a): echo rows badge the *cached*
/// conformance — a lookup, never a decode — each pole in its own words, and
/// a key the cache never checked renders the cache's own state: "not yet
/// checked", never a blank that could read as fine, never a borrowed
/// verdict, and never the "not checked" a check that ran can answer.
#[test]
fn the_echo_rows_badge_four_poles_and_admit_the_unchecked() {
    use std::time::Instant;
    use zengui::echo::EchoRing;
    use zengui::verdict::VerdictCache;
    use zengui::view::echo::{EchoView, section};
    use zenkey_fleet::SampleView;
    use zenkey_fleet::report::Conformance;

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
    let valid = "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0";
    let invalid = "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/wlan0";
    let undecodable = "zk2/robot1/nav/nav.v2/state/origin";
    let not_checked = "zk2/robot1/ghost/nav.v2/stream/position";
    let unchecked = "zk2/robot1/nav/nav.v2/state/status";
    let mut ring = EchoRing::new(10);
    for key in [valid, invalid, undecodable, not_checked, unchecked] {
        ring.push(&sample(key));
    }

    let mut verdicts = VerdictCache::default();
    verdicts.record(valid, Conformance::Valid);
    verdicts.record(
        invalid,
        Conformance::Invalid {
            violations: vec!["/is_up: \"yes\" is not of type \"boolean\"".into()],
        },
    );
    verdicts.record(
        undecodable,
        Conformance::Undecodable {
            declared: "nav.v2.Origin".into(),
            reason: "buffer underflow".into(),
        },
    );
    verdicts.record(
        not_checked,
        Conformance::NotChecked {
            reason: "no provider of this address in presence".into(),
        },
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
    for (word, why) in [
        ("valid", "checked and passed"),
        (
            "invalid (1 violation)",
            "checked, decoded, and failed its schema",
        ),
        (
            "undecodable (as nav.v2.Origin)",
            "not the declared type at all",
        ),
        (
            "not checked (no provider of this address in presence)",
            "a check that stopped at a rung, and says which",
        ),
        (
            zengui::verdict::UNCHECKED_LABEL,
            "a key the cache never checked — the cache's own state",
        ),
    ] {
        assert!(ui.find(word).is_ok(), "{why}: {word:?} not on screen");
    }
    assert!(
        ui.find("verdicts: 4 keys checked (each badge is the key's most recently checked sample)")
            .is_ok(),
        "the strip scopes what a badge claims"
    );
}

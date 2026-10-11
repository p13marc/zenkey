//! The nodes pane, the node detail and the presence row.

use super::*;

/// The nodes pane (#61): a retracted token reads suspect, the catalog is
/// named, and freshness never invents a verdict.
#[test]
fn the_nodes_pane_marks_retraction_suspect_and_names_the_catalog() {
    use zengui::nodes::NodeRoster;
    use zengui::view::nodes::{DetailState, NodesData, pane};

    let mut roster = NodeRoster::default();
    roster.apply_transitions(
        "",
        &[
            ("v1/h-3fa9c2d41b7e/state/sysinfo/alive".to_string(), true),
            ("v1/h-aaaaaaaaaaaa/state/other/alive".to_string(), true),
            ("v1/h-aaaaaaaaaaaa/state/other/alive".to_string(), false),
        ],
        Instant::now(),
    );

    let mut ui = simulator::<Message, _, _>(pane(NodesData {
        sp: sp(),
        roster: &roster,
        selected: None,
        detail: &DetailState::NotAsked,
        slices: None,
    }));
    assert!(
        ui.find("sysinfo: alive").is_ok(),
        "the live row reads alive"
    );
    assert!(
        ui.find("other: suspect since 0s (token retracted)").is_ok(),
        "retraction reads suspect immediately, never aged out"
    );
    assert!(
        ui.find("catalog: no token observed — not proof none exists (RFC 05 §3.1)")
            .is_ok(),
        "the catalog is asked for by name, never folded into 'no nodes'"
    );
    assert!(
        ui.find("not watched — freshness unknown").is_ok(),
        "an unwatched producer's freshness is unknown, not fresh (O4)"
    );
}

/// The nodes pane's O4 ladder before any presence source reports.
#[test]
fn the_nodes_pane_distinguishes_not_asked_from_empty() {
    use zengui::nodes::NodeRoster;
    use zengui::view::nodes::{DetailState, NodesData, pane};

    let roster = NodeRoster::default();
    let mut ui = simulator::<Message, _, _>(pane(NodesData {
        sp: sp(),
        roster: &roster,
        selected: None,
        detail: &DetailState::NotAsked,
        slices: None,
    }));
    assert!(
        ui.find("v1 presence not asked").is_ok(),
        "'not asked' must not render as an empty fleet"
    );
}

/// The node detail's freshness honesty: an unanswered sample is stale, and
/// zero ttl declarations is unknown, never fresh.
#[test]
fn the_node_detail_reports_freshness_honestly() {
    use std::sync::Arc;
    use zengui::nodes::NodeRoster;
    use zengui::view::nodes::{DetailState, NodesData, pane};
    use zenkey_fleet_v1::{Freshness, NodeInfo, ProducerInfo};

    let mut roster = NodeRoster::default();
    roster.apply_transitions(
        "",
        &[("v1/h-3fa9c2d41b7e/state/sysinfo/alive".to_string(), true)],
        Instant::now(),
    );
    let info = NodeInfo {
        origin: "h-3fa9c2d41b7e".to_string(),
        producers: vec![ProducerInfo {
            name: "sysinfo".to_string(),
            alive: true,
            app: Some("demo".to_string()),
            registry_version: Some("1.0".to_string()),
            subjects: 2,
            procedures: 1,
            blob_tiers: vec![],
            media: vec![],
            deprecated_served: 0,
            unreadable: None,
        }],
        freshness: vec![Freshness {
            producer: "sysinfo".to_string(),
            path: "health".to_string(),
            ttl_s: 30,
            age_s: None,
            stale: true,
        }],
    };
    let detail = DetailState::Loaded("h-3fa9c2d41b7e".to_string(), Ok(Arc::new(info)));
    let mut ui = simulator::<Message, _, _>(pane(NodesData {
        sp: sp(),
        roster: &roster,
        selected: Some("h-3fa9c2d41b7e"),
        detail: &detail,
        slices: None,
    }));
    assert!(
        ui.find("sysinfo/health  no sample answered — stale  (ttl 30s)  STALE")
            .is_ok(),
        "an unanswered ttl'd subject reads stale with its evidence"
    );
}

/// A producer that answered `introspect` with a slice that did not read is
/// drawn as that — the declared encoding and the error's first line — and
/// never as "no introspect reply" (#491; RFC 08 §6, v1.44; RFC 13 §3 O4).
#[test]
fn the_node_detail_says_unreadable_not_no_reply() {
    use std::sync::Arc;
    use zengui::nodes::NodeRoster;
    use zengui::view::nodes::{DetailState, NodesData, pane};
    use zenkey_fleet_v1::report::UnreadableSlice;
    use zenkey_fleet_v1::{NodeInfo, ProducerInfo};

    let mut roster = NodeRoster::default();
    roster.apply_transitions(
        "",
        &[("v1/h-3fa9c2d41b7e/state/tracker/alive".to_string(), true)],
        Instant::now(),
    );
    let row = |name: &str, unreadable: Option<UnreadableSlice>| ProducerInfo {
        name: name.to_string(),
        alive: true,
        app: None,
        registry_version: None,
        subjects: 0,
        procedures: 0,
        blob_tiers: vec![],
        media: vec![],
        deprecated_served: 0,
        unreadable,
    };
    let info = NodeInfo {
        origin: "h-3fa9c2d41b7e".to_string(),
        producers: vec![
            row(
                "tracker",
                Some(UnreadableSlice::new(
                    Some("application/json".into()),
                    &"unreadable registry slice: the reply declares encoding \"application/json\"\nsecond line",
                )),
            ),
            row("parallax", None),
        ],
        freshness: vec![],
    };
    let detail = DetailState::Loaded("h-3fa9c2d41b7e".to_string(), Ok(Arc::new(info)));
    let mut ui = simulator::<Message, _, _>(pane(NodesData {
        sp: sp(),
        roster: &roster,
        selected: Some("h-3fa9c2d41b7e"),
        detail: &detail,
        slices: None,
    }));
    assert!(
        ui.find(
            "tracker: introspect answered, slice unreadable (`application/json`: unreadable \
             registry slice: the reply declares encoding \"application/json\")"
        )
        .is_ok(),
        "answered-unreadable is its own sentence, first line only"
    );
    assert!(
        ui.find("parallax: no introspect reply — capabilities unknown, not absent")
            .is_ok(),
        "and no reply keeps its own"
    );
}

/// The presence row draws the engine's `node_rows` join, and a row whose
/// origin answered `introspect` with a slice that did not read says so in
/// the node detail's sentence — not the blank a producer with no slice
/// gets (#495; RFC 08 §6, v1.44; RFC 13 §3 O4).
#[test]
fn the_presence_row_says_unreadable_not_blank() {
    use zengui::nodes::NodeRoster;
    use zengui::view::nodes::presence_section;
    use zenkey_fleet_v1::report::{NodeList, NodeRow, UnreadableSlice};

    let origin = "h-3fa9c2d41b7e";
    let mut roster = NodeRoster::default();
    roster.apply_transitions(
        "",
        &[
            (format!("v1/{origin}/state/tracker/alive"), true),
            (format!("v1/{origin}/state/parallax/alive"), true),
        ],
        Instant::now(),
    );
    let row = |producer: &str, unreadable: Option<UnreadableSlice>| NodeRow {
        origin: origin.to_string(),
        producer: producer.to_string(),
        app: None,
        registry_version: None,
        unreadable,
    };
    let joined = NodeList {
        slices_joined: true,
        nodes: vec![
            row(
                "tracker",
                Some(UnreadableSlice::new(
                    Some("application/json".into()),
                    &"unreadable registry slice: the reply declares encoding \"application/json\"\nsecond line",
                )),
            ),
            row("parallax", None),
        ],
    };
    let mut ui =
        simulator::<Message, _, _>(presence_section(&roster, origin, Some(&joined), &[], sp()));
    assert!(
        ui.find(
            "introspect answered, slice unreadable (`application/json`: unreadable registry \
             slice: the reply declares encoding \"application/json\")"
        )
        .is_ok(),
        "answered-unreadable is the detail section's sentence, first line only"
    );
    assert!(
        ui.find("parallax: alive").is_ok(),
        "and the producer with no slice still draws its presence"
    );
}

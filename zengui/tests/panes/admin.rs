//! The admin & storage panel's honesty surfaces (#70). The pane exists to draw
//! three distinctions that all look like "nothing here" unless something makes
//! them look different: not swept, swept-and-unreachable, and swept-and-empty.

use super::*;
use std::sync::Arc;
use zengui::admin::{AdminState, AdminSweep};
use zengui::view::admin::pane;
use zenkey_fleet_v1::report::StorageList;
use zenkey_fleet_v1::{Coverage, CoverageRow, DeclaredEntities, RouterInfo, StorageInfo};

fn sweep(
    routers: Vec<RouterInfo>,
    storages: Vec<StorageInfo>,
    coverage: Vec<CoverageRow>,
    declared: Option<DeclaredEntities>,
    note: Option<&str>,
) -> AdminState {
    let mut state = AdminState::default();
    state.finish(
        Ok(Arc::new(AdminSweep {
            routers,
            storage: StorageList { storages, coverage },
            declared,
            coverage_note: note.map(str::to_string),
            topology: zenkey_fleet_v1::TopologyReport {
                nodes: vec![],
                edges: vec![],
                asked: "@/*/*".into(),
                answered: 0,
                self_zid: String::new(),
            },
            origins: vec![],
            base: String::new(),
        })),
        "",
    );
    state
}

/// #118: the topology caption is the testable surface (the canvas is
/// find-opaque) — it names the heard-of nodes, the storage hosts, and
/// this session, and an unanswered admin space stays a reachability
/// reading, never an empty mesh.
#[test]
fn the_topology_caption_carries_the_overlay_facts() {
    let mut state = AdminState::default();
    state.finish(
        Ok(Arc::new(AdminSweep {
            routers: vec![],
            storage: StorageList {
                storages: vec![storage("s1")],
                coverage: vec![],
            },
            declared: None,
            coverage_note: None,
            topology: zenkey_fleet_v1::TopologyReport {
                nodes: vec![
                    zenkey_fleet_v1::TopologyNode {
                        zid: "z1".into(),
                        whatami: "router".into(),
                        version: Some("1.9.0".into()),
                        locators: vec!["tcp/10.0.0.1:7447".into()],
                        locators_via_links: vec![],
                        answered: true,
                    },
                    zenkey_fleet_v1::TopologyNode {
                        zid: "z2".into(),
                        whatami: "peer".into(),
                        version: None,
                        locators: vec![],
                        locators_via_links: vec![],
                        answered: false,
                    },
                ],
                edges: vec![
                    zenkey_fleet_v1::TopologyEdge {
                        reporter: "z1".into(),
                        peer: "z2".into(),
                        whatami: "peer".into(),
                        region: None,
                        links: vec![],
                    },
                    // The same link from the other end: corroboration,
                    // not duplication (`mesh_links`, #234).
                    zenkey_fleet_v1::TopologyEdge {
                        reporter: "z2".into(),
                        peer: "z1".into(),
                        whatami: "router".into(),
                        region: None,
                        links: vec![],
                    },
                ],
                asked: "@/*/*".into(),
                answered: 1,
                self_zid: "z2".into(),
            },
            origins: vec![
                zenkey_fleet_v1::OriginAttachment {
                    origin: "h-cccccccccccc".into(),
                    session_zid: Some("z2".into()),
                    reporter_zid: "z1".into(),
                    token_key: "v1/h-cccccccccccc/state/demo/alive".into(),
                },
                zenkey_fleet_v1::OriginAttachment {
                    origin: "h-dddddddddddd".into(),
                    session_zid: None,
                    reporter_zid: "z1".into(),
                    token_key: "v1/h-dddddddddddd/state/demo/alive".into(),
                },
                zenkey_fleet_v1::OriginAttachment {
                    origin: "h-eeeeeeeeeeee".into(),
                    session_zid: Some("z-not-drawn".into()),
                    reporter_zid: "z9".into(),
                    token_key: "v1/h-eeeeeeeeeeee/state/demo/alive".into(),
                },
            ],
            base: String::new(),
        })),
        "",
    );
    let mut ui = simulator::<Message, _, _>(zengui::view::mesh::pane(&state, sp()));
    // #131: the origin join's evidence distinction reaches the caption —
    // attached-by-declaration vs reported-only vs outside the mesh.
    assert!(
        ui.find(
            "origins: 1 attached by declaration · 1 reported-only (dotted) · 1 naming a session outside the drawn mesh (listed, not drawn)"
        )
        .is_ok(),
        "the three evidence grades are counted out loud"
    );
    assert!(
        ui.find("drag to pan · scroll to zoom · right-click resets")
            .is_ok(),
        "the viewport controls are discoverable"
    );
    // #234: the engine's `mesh_links` dedup keeps the evidence grade the
    // hand-rolled one threw away, and the caption states it.
    assert!(
        ui.find(
            "1 of 1 link(s) corroborated by both ends — a link only one end mentions is weaker evidence (drawn thinner)"
        )
        .is_ok(),
        "reciprocal reports read as corroboration, not as two links"
    );
    assert!(
        ui.find("copy graphviz (dot)").is_ok(),
        "the render_dot export is offered where the mesh is (#234)"
    );
    // iced_test's find matches a widget's WHOLE text.
    assert!(
        ui.find("z1  router  ⌂ storage").is_ok(),
        "the answered node rolls, with the storage overlay marking its host"
    );
    assert!(
        ui.find("z2  peer  (heard of)  ← you").is_ok(),
        "heard-of and you-are-here ride the caption"
    );

    // And the unanswered mesh stays a reading.
    let empty = sweep(vec![], vec![], vec![], None, None);
    let mut ui = simulator::<Message, _, _>(zengui::view::mesh::pane(&empty, sp()));
    assert!(
        ui.find(
            "no admin space answered @/*/* — adminspace.enabled defaults off; this is a \
         reading about reachability, never an empty mesh (RFC 05 §3.1)"
        )
        .is_ok(),
        "zero answers is reachability, not topology"
    );
}

fn storage(name: &str) -> StorageInfo {
    StorageInfo {
        zid: "z1".into(),
        name: name.into(),
        key_expr: Some("acme/v1/*/state/**".into()),
        strip_prefix: Some("acme/v1".into()),
        volume: Some("rocksdb".into()),
        raw: serde_json::json!({"volume": "rocksdb"}),
    }
}

fn row(producer: &str, path: &str, coverage: Coverage) -> CoverageRow {
    CoverageRow {
        producer: producer.into(),
        path: path.into(),
        ttl_s: Some(60),
        coverage,
    }
}

/// O4: never swept must not read as "no routers".
#[test]
fn the_admin_pane_never_reports_a_sweep_it_did_not_run() {
    let state = AdminState::default();
    let mut ui = simulator::<Message, _, _>(pane(&state, sp()));
    assert!(ui.find("admin space not swept yet").is_ok());
    assert!(
        ui.find(
            "nothing has been asked — this is \"not asked\", not \"no routers\" \
             (RFC 09 §5.1 O4)"
        )
        .is_ok()
    );
    assert!(
        ui.find(
            "no routers answered @/*/router — a peer-only mesh, or the admin space \
                 is disabled."
        )
        .is_err(),
        "an unrun sweep must not render the swept-and-empty sentence"
    );
}

/// #70's second acceptance criterion: a router-less bus shows the honest
/// empty states, not an error — and above all not a coverage verdict.
#[test]
fn a_router_less_bus_says_so_four_ways() {
    let state = sweep(
        vec![],
        vec![],
        vec![],
        None,
        Some(
            "no registry slices resolved, so the declared state families are unknown — \
             this is \"not asked\", not \"uncovered\" (RFC 09 §5.1 O4). Pass --registry, \
             or wait for the bus registry (RFC 08 §6).",
        ),
    );
    let mut ui = simulator::<Message, _, _>(pane(&state, sp()));
    // Verbatim from the CLI, so the two tools cannot disagree.
    assert!(
        ui.find(
            "no routers answered @/*/router — a peer-only mesh, or the admin space is \
             disabled."
        )
        .is_ok()
    );
    assert!(
        ui.find(
            "no storages found in the admin space — a peer-only mesh, a router without \
             the storage manager, or the admin space is disabled."
        )
        .is_ok()
    );
    assert!(ui.find("coverage not judged").is_ok());
    assert!(
        ui.find(
            "declared entities: n/a — nothing answered the admin sweep. zenoh's \
             adminspace.enabled defaults to false and a peer mesh has none, so this is \
             \"not available\", never \"nothing declared\" (RFC 09 §5.1 O4)."
        )
        .is_ok()
    );
    // The negative that matters: an unjudged table must never render as an
    // uncovered one.
    assert!(
        ui.find("uncovered").is_err(),
        "coverage that was never judged must not read as uncovered"
    );
}

/// A populated sweep: the three coverage verdicts read differently, both
/// RFC notes are present, and the new storage fields are on screen.
#[test]
fn a_populated_sweep_renders_every_verdict_distinctly() {
    let state = sweep(
        vec![RouterInfo {
            zid: "z1".into(),
            version: Some("1.9.0".into()),
            locators: vec!["tcp/127.0.0.1:7447".into()],
            raw: serde_json::Value::Null,
        }],
        vec![storage("latest")],
        vec![
            row("tc", "health", Coverage::Covered("latest@z1".into())),
            row("tc", "config/{iface}", Coverage::Partial("one@z1".into())),
            row("netring", "capture", Coverage::Uncovered),
        ],
        Some(DeclaredEntities::default()),
        None,
    );
    let mut ui = simulator::<Message, _, _>(pane(&state, sp()));
    assert!(ui.find("covered by latest@z1").is_ok());
    assert!(ui.find("PARTIAL via one@z1").is_ok());
    assert!(ui.find("uncovered").is_ok());
    // The consequence RFC 09 §2 names, which is the visual #70 asked for.
    assert!(
        ui.find(
            "no storage captures an uncovered family — while its producer is down, a \
             late joiner GETs nothing: the `latest` storage is the fleet-wide \
             late-joiner seed (RFC 09 §2)"
        )
        .is_ok()
    );
    // And the counterweight, verbatim from the CLI.
    assert!(
        ui.find(
            "note: an uncovered ttl'd family is not automatically a defect — \
             volatile-state seeding may ride the advanced-pub/sub cache (RFC 04 §3.5); \
             storage is authoritative for durable data."
        )
        .is_ok()
    );
    assert!(
        ui.find("strip acme/v1  ·  volume rocksdb").is_ok(),
        "the fields #70 asked for reach the screen"
    );
    assert!(
        ui.find("ttl 60s").is_ok(),
        "the note is unusable without it"
    );
}

/// The O4 pair, pinned: "answered and declared none" must not render as
/// "nothing answered".
#[test]
fn an_empty_admin_space_reads_differently_from_an_absent_one() {
    let state = sweep(
        vec![],
        vec![],
        vec![],
        Some(DeclaredEntities::default()),
        None,
    );
    let mut ui = simulator::<Message, _, _>(pane(&state, sp()));
    assert!(
        ui.find("declared entities: 0 — the admin space answered and declared none")
            .is_ok()
    );
    assert!(
        ui.find(
            "declared entities: n/a — nothing answered the admin sweep. zenoh's \
             adminspace.enabled defaults to false and a peer mesh has none, so this is \
             \"not available\", never \"nothing declared\" (RFC 09 §5.1 O4)."
        )
        .is_err(),
        "an answered sweep must not carry the unreachable sentence"
    );
}

//! The admin & storage panel's honesty surfaces (#70; zk2's since #614,
//! FL9). The pane exists to draw three distinctions that all look like
//! "nothing here" unless something makes them look different: not swept,
//! swept-and-unreachable, and swept-and-empty — and, since S4, a storage on
//! an owner's state as the finding it is.

use super::*;
use std::sync::Arc;
use zengui::admin::{AdminState, AdminSweep};
use zengui::view::admin::{DECLARED_OFF, Declared, pane};
use zenkey_fleet::DeclaredEntities;
use zenkey_fleet::report::{
    Asked, AttachedTo, Attachment, InstanceAttachment, InstanceJoin, RouterInfo, StorageInfo,
    TopologyEdge, TopologyNode, TopologyReport,
};

fn topology(nodes: Vec<TopologyNode>, edges: Vec<TopologyEdge>) -> TopologyReport {
    let answered = nodes.iter().filter(|n| n.answered).count();
    TopologyReport {
        nodes,
        edges,
        asked: "@/*/*".into(),
        answered,
        self_zid: "z2".into(),
        instances: Asked::NotAsked,
    }
}

fn sweep(
    namespace: &str,
    routers: Vec<RouterInfo>,
    storages: Vec<StorageInfo>,
    topology: TopologyReport,
) -> AdminState {
    let mut state = AdminState::default();
    let s4 = zengui::admin::s4_rows(namespace, &storages);
    state.finish(
        Ok(Arc::new(AdminSweep {
            routers,
            storages,
            s4,
            topology,
            namespace: namespace.into(),
        })),
        namespace,
    );
    state
}

fn node(zid: &str, whatami: &str, answered: bool) -> TopologyNode {
    TopologyNode {
        zid: zid.into(),
        whatami: whatami.into(),
        version: answered.then(|| "1.10.1".into()),
        locators: if answered {
            vec!["tcp/10.0.0.1:7447".into()]
        } else {
            vec![]
        },
        locators_via_links: vec![],
        answered,
    }
}

fn edge(reporter: &str, peer: &str, whatami: &str) -> TopologyEdge {
    TopologyEdge {
        reporter: reporter.into(),
        peer: peer.into(),
        whatami: whatami.into(),
        region: None,
        links: vec![],
    }
}

fn instance(address: &str, attachment: Attachment) -> InstanceAttachment {
    InstanceAttachment {
        address: address.into(),
        instance: "0000000000000001".into(),
        zid: Some("ab12".into()),
        attachment,
    }
}

/// #118: the topology caption is the testable surface (the canvas is
/// find-opaque) — it names the heard-of nodes, the storage hosts, this
/// session and zk2's instances joined onto the routers (#705), attached or
/// not; and an unanswered admin space stays a reachability reading, never
/// an empty mesh.
#[test]
fn the_topology_caption_carries_the_overlay_facts() {
    let mut topo = topology(
        vec![node("z1", "router", true), node("z2", "peer", false)],
        vec![
            edge("z1", "z2", "peer"),
            // The same link from the other end: corroboration, not
            // duplication (`mesh_links`, #234).
            edge("z2", "z1", "router"),
        ],
    );
    topo.instances = Asked::Asked(InstanceJoin {
        namespace: "acme".into(),
        selector: "zk2/*/*/@zk/**".into(),
        complete: true,
        cut_off_during: false,
        verified: vec!["z1".into()],
        unverified: vec![],
        unobservable: None,
        instances: vec![
            instance(
                "robot1/nav",
                Attachment::Attached {
                    routers: vec![AttachedTo {
                        router: "z1".into(),
                        listed_as: "client".into(),
                    }],
                },
            ),
            instance(
                "host-a/tc",
                Attachment::Unattached {
                    reason: "no verified router lists ab12".into(),
                },
            ),
            instance(
                "robot9/phantom",
                Attachment::Unattributable {
                    reason: "no descriptor read".into(),
                },
            ),
        ],
    });
    let mut storage = storage("s1", Some("demo/**"));
    storage.zid = "z1".into();
    let state = sweep("acme", vec![], vec![storage], topo);
    let mut ui = simulator::<Message, _, _>(zengui::view::mesh::pane(&state, sp()));
    assert!(
        ui.find(
            "instances: 1 attached to a verified router (drawn) · 1 unattached · 1 \
             unattributable (no zid) · 0 on a router outside the drawing — listed, not drawn"
        )
        .is_ok(),
        "every instance presence showed is counted, attached or not"
    );
    assert!(
        ui.find("drag to pan · scroll to zoom · right-click resets")
            .is_ok(),
        "the viewport controls are discoverable"
    );
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

    // Without a session in the namespace the join is not asked — said so.
    let bare = sweep(
        "acme",
        vec![],
        vec![],
        topology(vec![node("z1", "router", true)], vec![]),
    );
    let mut ui = simulator::<Message, _, _>(zengui::view::mesh::pane(&bare, sp()));
    assert!(
        ui.find("instances: not joined — no session in the namespace to read presence through")
            .is_ok()
    );

    // And the unanswered mesh stays a reading.
    let empty = sweep("acme", vec![], vec![], topology(vec![], vec![]));
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

fn storage(name: &str, key_expr: Option<&str>) -> StorageInfo {
    StorageInfo {
        zid: "z1".into(),
        name: name.into(),
        key_expr: key_expr.map(Into::into),
        strip_prefix: Some("acme".into()),
        volume: Some("rocksdb".into()),
        raw: serde_json::json!({"volume": "rocksdb"}),
    }
}

/// O4: never swept must not read as "no routers".
#[test]
fn the_admin_pane_never_reports_a_sweep_it_did_not_run() {
    let state = AdminState::default();
    let mut ui = simulator::<Message, _, _>(pane(&state, Declared::Off, sp()));
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
/// empty states, not an error — and no S4 verdict about storages it never
/// saw.
#[test]
fn a_router_less_bus_says_so_four_ways() {
    let state = sweep("", vec![], vec![], topology(vec![], vec![]));
    let mut ui = simulator::<Message, _, _>(pane(&state, Declared::Unanswered, sp()));
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
    assert!(
        ui.find("no storage to judge — the sweep found none (above)")
            .is_ok()
    );
    assert!(
        ui.find(
            "declared entities: n/a — not read yet, or nothing answered. zenoh's \
             adminspace.enabled defaults to false and a peer mesh has none, so this is \
             \"not available\", never \"nothing declared\" (RFC 09 §5.1 O4)."
        )
        .is_ok()
    );
    assert!(
        ui.find("not on owner state").is_err(),
        "no storage, no clean row"
    );
}

/// The S4 table (D14): a storage on an owner's state is the finding, one
/// beside it clean, one whose document names no key expression not judged
/// — three rows, three words and marks — and the rule said once. v1's
/// "covered" and its late-joiner-seed sentence are gone.
#[test]
fn the_s4_table_reads_a_storage_on_owner_state_as_the_finding() {
    let state = sweep(
        "acme",
        vec![RouterInfo {
            zid: "z1".into(),
            version: Some("1.10.1".into()),
            locators: vec!["tcp/127.0.0.1:7447".into()],
            raw: serde_json::Value::Null,
        }],
        vec![
            storage("state", Some("acme/zk2/**")),
            storage("events", Some("acme/zk2/*/*/*/events/**")),
            storage("mystery", None),
        ],
        topology(vec![node("z1", "router", true)], vec![]),
    );
    let declared = DeclaredEntities::default();
    let mut ui = simulator::<Message, _, _>(pane(&state, Declared::Answered(&declared), sp()));
    assert!(ui.find("S4: storages on owners' state").is_ok());
    assert!(ui.find("answers on owner state").is_ok());
    assert!(ui.find("intersects acme/zk2/*/*/*/state/**").is_ok());
    assert!(ui.find("not on owner state").is_ok());
    assert!(ui.find("not judged").is_ok());
    assert!(ui.find("its admin document names no key_expr").is_ok());
    assert!(
        ui.find(
            "a deployment MUST NOT run a storage on owners' state (core §4.2 S4): a late \
             joiner seeds from the owner, and last-known state is an archive.v1's (§4.4)"
        )
        .is_ok(),
        "the rule, said once"
    );
    assert!(
        ui.find("strip acme  ·  volume rocksdb").is_ok(),
        "the storage's own fields still reach the screen"
    );
    assert!(
        ui.find("covered by latest@z1").is_err(),
        "v1's coverage reading is gone"
    );
}

/// The O4 pair, pinned: "answered and declared none" must not render as
/// "nothing answered".
#[test]
fn an_empty_admin_space_reads_differently_from_an_absent_one() {
    let state = sweep("", vec![], vec![], topology(vec![], vec![]));
    let declared = DeclaredEntities::default();
    let mut ui = simulator::<Message, _, _>(pane(&state, Declared::Answered(&declared), sp()));
    assert!(
        ui.find("declared entities: 0 — the admin space answered and declared none")
            .is_ok()
    );
    assert!(
        ui.find(
            "declared entities: n/a — not read yet, or nothing answered. zenoh's \
             adminspace.enabled defaults to false and a peer mesh has none, so this is \
             \"not available\", never \"nothing declared\" (RFC 09 §5.1 O4)."
        )
        .is_err(),
        "an answered read must not carry the unreachable sentence"
    );
}

/// The declared entities off (#890), the default: the pane says they were
/// not asked and why, and never reads as "nothing answered" or "none".
#[test]
fn a_declared_sweep_that_is_off_says_not_asked_and_why() {
    let state = sweep("", vec![], vec![], topology(vec![], vec![]));
    let mut ui = simulator::<Message, _, _>(pane(&state, Declared::Off, sp()));
    assert!(ui.find(DECLARED_OFF).is_ok());
    assert!(DECLARED_OFF.contains("#890") && DECLARED_OFF.contains("--admin-declared"));
    assert!(
        ui.find("declared entities: 0 — the admin space answered and declared none")
            .is_err()
    );
    assert!(
        ui.find(
            "declared entities: n/a — not read yet, or nothing answered. zenoh's \
             adminspace.enabled defaults to false and a peer mesh has none, so this is \
             \"not available\", never \"nothing declared\" (RFC 09 §5.1 O4)."
        )
        .is_err(),
        "not asked is not \"nothing answered\""
    );
    // An answer held when the switch went off is still shown: it is what
    // was read.
    let declared = DeclaredEntities::default();
    assert!(matches!(
        Declared::of(false, Some(&declared)),
        Declared::Answered(_)
    ));
    assert!(matches!(Declared::of(true, None), Declared::Unanswered));
    assert!(matches!(Declared::of(false, None), Declared::Off));
}

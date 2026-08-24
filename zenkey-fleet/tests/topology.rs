//! The topology join (#118): admin root docs become nodes, their session
//! lists become edges, and a mentioned-but-silent node renders "heard of,
//! not queryable" rather than being omitted.
//!
//! Since zenoh 1.10 the root doc filters loopback endpoints out of its
//! `locators` (eclipse-zenoh/zenoh#2671), so this loopback-bound fixture
//! also pins the corroboration path: the listen endpoint reaches the
//! report as `locators_via_links` — link evidence, not a listen claim
//! (#155).
//!
//! The fixture dogfoods #122: `adminspace.enabled` defaults to false and
//! `session::open` never turns it on, so the serving peer is opened through
//! the config passthrough with a file that enables it — exactly how an
//! operator would.
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::time::Duration;

mod util;
use util::{admin_config, endpoint};

/// A mesh where one peer serves its admin space: the join names the server
/// as answered, the other peer as an edge — and as a heard-of node, since
/// its own admin space never answered.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_answering_peer_becomes_a_node_and_its_sessions_become_edges() {
    let file = admin_config();
    let endpoint = endpoint();
    let serving = zenkey_fleet::open_with_config(
        Some(&file),
        &[],
        std::slice::from_ref(&endpoint),
        Some(false),
    )
    .await
    .expect("serving session");
    let asking = zenkey_fleet::session::open(std::slice::from_ref(&endpoint), &[], false)
        .await
        .expect("asking session");
    // The listen address as a locator spells it, for the link evidence below.
    let listen_addr = endpoint.trim_start_matches("tcp/").to_string();

    // Settle: loop until the admin space answers (wait-routable).
    let report = loop {
        let report = zenkey_fleet::topology(&asking, Duration::from_millis(500))
            .await
            .expect("a quiet admin space is a reading, not an error");
        if report.answered > 0 {
            break report;
        }
    };

    let serving_zid = serving.zid().to_string();
    let asking_zid = asking.zid().to_string();
    assert_eq!(report.self_zid, asking_zid, "the you-are-here marker");

    let server = report
        .nodes
        .iter()
        .find(|n| n.zid == serving_zid)
        .expect("the serving peer answered as a node");
    assert!(server.answered);
    assert_eq!(server.whatami, "peer");
    // zenoh 1.10 filters loopback endpoints out of the root doc
    // (eclipse-zenoh/zenoh#2671 — deliberate, the loopback scouting fix),
    // and this fixture listens on 127.0.0.1 only, so the honest root-doc
    // answer is *no locators*. If this assertion ever fails, upstream
    // changed its mind about the filter — re-read #155 before trusting
    // root-doc locators again.
    assert!(
        server.locators.is_empty(),
        "a loopback-only 1.10 node declares no root-doc locators: {:?}",
        server.locators
    );
    // The join corroborates from the session link instead: the server-side
    // endpoint of the reported link is the listen address — labelled link
    // evidence, kept out of `locators`, never an invented listen claim.
    assert!(
        server
            .locators_via_links
            .iter()
            .any(|l| l.contains(&listen_addr)),
        "the listen endpoint rides out as link evidence: {:?}",
        server.locators_via_links
    );

    let edge = report
        .edges
        .iter()
        .find(|e| e.reporter == serving_zid && e.peer == asking_zid)
        .unwrap_or_else(|| {
            panic!(
                "the server reports its session to the asker as an edge: {:?}",
                report.edges
            )
        });
    // 1.10 session entries declare a region (the regions rework); carried
    // verbatim as data, whatever it says.
    assert!(
        edge.region.is_some(),
        "a 1.10 session entry states its region: {edge:?}"
    );
    let heard_of = report
        .nodes
        .iter()
        .find(|n| n.zid == asking_zid)
        .expect("the asker is mentioned, so it is a node");
    assert!(
        !heard_of.answered,
        "the asker's own admin space is off: heard of, not queryable — never omitted"
    );

    std::fs::remove_file(file).ok();
}

/// An admin-less mesh answers an empty topology — a reading about
/// reachability, never an error and never an invented mesh.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_admin_less_mesh_is_a_reading_not_a_mesh() {
    let endpoint = endpoint();
    let _listen = zenkey_fleet::session::open(&[], std::slice::from_ref(&endpoint), false)
        .await
        .expect("listener");
    let asking = zenkey_fleet::session::open(std::slice::from_ref(&endpoint), &[], false)
        .await
        .expect("asker");
    let report = zenkey_fleet::topology(&asking, Duration::from_millis(400))
        .await
        .expect("silence is a reading");
    assert_eq!(report.answered, 0);
    assert!(report.edges.is_empty());
}

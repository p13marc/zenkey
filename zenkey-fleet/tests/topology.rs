//! The topology join (#118): admin root docs become nodes, their session
//! lists become edges, and a mentioned-but-silent node renders "heard of,
//! not queryable" rather than being omitted.
//!
//! The fixture dogfoods #122: `adminspace.enabled` defaults to false and
//! `session::open` never turns it on, so the serving peer is opened through
//! the config passthrough with a file that enables it — exactly how an
//! operator would. Ports 7522-7523 (disjoint from every other test binary).

use std::time::Duration;

fn admin_config() -> std::path::PathBuf {
    let path = std::env::temp_dir().join("zenkey-fleet-topology-admin.json5");
    std::fs::write(&path, r#"{ adminspace: { enabled: true } }"#).unwrap();
    path
}

/// A mesh where one peer serves its admin space: the join names the server
/// as answered, the other peer as an edge — and as a heard-of node, since
/// its own admin space never answered.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_answering_peer_becomes_a_node_and_its_sessions_become_edges() {
    let file = admin_config();
    let serving = zenkey_fleet::open_with_config(
        Some(&file),
        &[],
        &["tcp/127.0.0.1:7522".to_string()],
        Some(false),
    )
    .await
    .expect("serving session");
    let asking = zenkey_fleet::session::open(&["tcp/127.0.0.1:7522".to_string()], &[], false)
        .await
        .expect("asking session");

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
    assert!(
        server.locators.iter().any(|l| l.contains("7522")),
        "locators ride out: {:?}",
        server.locators
    );

    assert!(
        report
            .edges
            .iter()
            .any(|e| e.reporter == serving_zid && e.peer == asking_zid),
        "the server reports its session to the asker as an edge: {:?}",
        report.edges
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
    let _listen = zenkey_fleet::session::open(&[], &["tcp/127.0.0.1:7523".to_string()], false)
        .await
        .expect("listener");
    let asking = zenkey_fleet::session::open(&["tcp/127.0.0.1:7523".to_string()], &[], false)
        .await
        .expect("asker");
    let report = zenkey_fleet::topology(&asking, Duration::from_millis(400))
        .await
        .expect("silence is a reading");
    assert_eq!(report.answered, 0);
    assert!(report.edges.is_empty());
}

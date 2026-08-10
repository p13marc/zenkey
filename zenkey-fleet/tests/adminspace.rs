//! The admin space against a mesh that has none (issue #70).
//!
//! `admin.rs` has claimed since it was written that zero routers is "an empty
//! vec, **never** an error", and that a sweep answering nothing is "not
//! available" rather than "nothing declared". Both claims are load-bearing —
//! the whole admin panel's empty states rest on them — and neither had a test,
//! because a peer pair has no storage manager and there was nothing to assert
//! *against*.
//!
//! That is exactly what makes a peer pair the right fixture: zenoh's
//! `adminspace.enabled` defaults to false and `session::open` never turns it
//! on, so this is the deployment an operator most often runs an explorer
//! against, and the one where a panic or an `Err` would be worst.
//!
//! Self-contained: in-process peers, explicit endpoints, no scouting, no
//! router. Ports 7513-7514 (disjoint from every other test binary).

use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(2);

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

/// Every admin call degrades to a *reading*, never to an error — which is what
/// lets an explorer render "peer-only mesh" instead of a failure dialog.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_admin_less_mesh_answers_empty_never_an_error() {
    let (_serving, asking) = peer_pair(7513).await;

    let routers = zenkey_fleet::routers(&asking, TIMEOUT)
        .await
        .expect("zero routers is a reading, not an error");
    // If this build *does* serve a peer admin space, the rows must still be
    // well-formed — the zid fallback reads chunk 1 of the key.
    for r in &routers {
        assert!(!r.zid.is_empty(), "a router row always names its node");
    }

    let storages = zenkey_fleet::storages(&asking, TIMEOUT)
        .await
        .expect("zero storages is a reading, not an error");
    assert!(
        storages.is_empty(),
        "an in-process peer cannot host a storage manager: {storages:?}"
    );

    // The `Option` the admin panel's reachability sentence rests on.
    let declared = zenkey_fleet::declared_entities(&asking, TIMEOUT)
        .await
        .expect("an unreachable admin space is a reading, not an error");
    match declared {
        None => { /* nothing answered — the expected reading here */ }
        Some(e) => {
            // Tolerated rather than asserted against: if a future zenoh serves
            // a peer admin space, "answered with entities" is still a valid
            // reading. What must never happen is an `Err`.
            for entity in &e.entities {
                assert!(!entity.keyexpr.is_empty());
            }
        }
    }
}

/// The coverage join is pure, so it holds the same shape with no storages at
/// all: every declared state family is judged, and judged `Uncovered` — which
/// is a *fact about coverage*, distinct from the panel's "not judged" state.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coverage_with_no_storages_judges_rather_than_abstains() {
    let (_serving, asking) = peer_pair(7514).await;
    let storages = zenkey_fleet::storages(&asking, TIMEOUT).await.unwrap();

    let slice = zenkey::parse_slice(
        r#"
        [registry]
        version = "1.0"
        app = "t"
        convention = 1
        [producer]
        name = "tc"
        [[subject]]
        path = "health"
        class = "state"
        type = "Health"
        ttl_s = 60
        [[subject]]
        path = "cpu"
        class = "telemetry"
        type = "Point"
        "#,
    )
    .expect("fixture slice");
    let slices = zenkey_fleet::SliceSet::from_slices(vec![slice]);
    let rows = zenkey_fleet::state_coverage(&slices, "", &storages);

    assert_eq!(rows.len(), 1, "state subjects only, never telemetry");
    assert_eq!(rows[0].path, "health");
    assert!(matches!(
        rows[0].coverage,
        zenkey_fleet::Coverage::Uncovered
    ));
    assert_eq!(
        rows[0].ttl_s,
        Some(60),
        "the ttl is what makes the RFC 04 §3.5 note readable"
    );
}

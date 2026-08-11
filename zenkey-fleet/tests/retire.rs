//! The retire path (#115, RFC 04 §1.2): a tombstone is a `SampleKind::Delete`
//! riding the declared publisher — never a payload marker, never an ad-hoc
//! `session.delete`.
//!
//! Event-driven like `matching.rs`: the publication's matching badge proves
//! the subscriber is routable before anything is published, so no sleep races
//! the fixture. Ports 7515-7516 (disjoint from every other test binary).

use std::time::Duration;

use zenkey::qos::QosProfile;
use zenkey_fleet::declare_publication;
use zenoh::sample::SampleKind;

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

const KEY: &str = "v1/h-dddddddddddd/state/demo/health";

/// `Publication::retire` reaches a raw subscriber as `SampleKind::Delete`,
/// after an ordinary put on the same declared publisher.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tombstone_reaches_the_subscriber_as_delete() {
    let (a, b) = peer_pair(7515).await;

    let publication = declare_publication(&a, KEY, QosProfile::Transition, None)
        .await
        .expect("declare publication");
    let events = publication.matching_events().await.expect("events");
    let subscriber = b.declare_subscriber(KEY).await.expect("subscriber");
    // The badge is the routability proof: no publish before it flips.
    let matched = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("matching event within 5s")
        .expect("listener alive");
    assert!(matched);

    publication
        .send(b"{\"ok\":true}".to_vec(), None)
        .await
        .expect("put");
    publication.retire().await.expect("retire");
    publication.undeclare().await.expect("undeclare");

    let put = tokio::time::timeout(Duration::from_secs(5), subscriber.recv_async())
        .await
        .expect("put within 5s")
        .expect("subscriber alive");
    assert_eq!(put.kind(), SampleKind::Put);
    let del = tokio::time::timeout(Duration::from_secs(5), subscriber.recv_async())
        .await
        .expect("delete within 5s")
        .expect("subscriber alive");
    assert_eq!(
        del.kind(),
        SampleKind::Delete,
        "a retirement is a delete on the wire, not an empty put"
    );
    assert!(del.payload().is_empty(), "a tombstone carries no payload");
}

/// The Monitor path reports the tombstone as `SampleView.kind == Delete` —
/// what the explorers render as retirement (they must be able to *see* what
/// `topic retire` just produced).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_monitor_reports_kind_delete() {
    let (a, b) = peer_pair(7516).await;

    let monitor = zenkey_fleet::Monitor::start(&b, zenkey_fleet::MonitorSpec::default())
        .await
        .expect("monitor");
    let mut events = monitor.events();
    monitor.watch(KEY).await.expect("watch");

    let publication = declare_publication(&a, KEY, QosProfile::Transition, None)
        .await
        .expect("declare publication");
    let matching = publication.matching_events().await.expect("events");
    let matched = tokio::time::timeout(Duration::from_secs(5), matching.recv())
        .await
        .expect("matching event within 5s")
        .expect("listener alive");
    assert!(matched, "the monitor's subscriber must become routable");

    publication.retire().await.expect("retire");

    let view = loop {
        let item = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("event within 5s")
            .expect("stream alive");
        if let zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::Sample(s)) = item {
            break s;
        }
    };
    assert_eq!(view.key, KEY);
    assert_eq!(
        view.kind,
        SampleKind::Delete,
        "SampleView carries the kind exactly (RFC 04 §1.2)"
    );
}

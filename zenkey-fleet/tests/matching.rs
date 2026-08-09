//! Matching badges on self-declared entities (#38, RFC 12 §9's allowed
//! half): a [`Publication`] knows when a subscriber matches it, a
//! [`RepeatingQuery`] knows when a queryable serves it — and neither claim
//! reaches beyond the entity this process declared.
//!
//! Event-driven like `lazy.rs`: the listener observes the transition, no
//! status poll races the declaration. Ports 7485-7486.

use std::time::Duration;

use zenkey::qos::QosProfile;
use zenkey_fleet::{declare_publication, declare_repeating};

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

const KEY: &str = "v1/h-cccccccccccc/state/demo/health";

/// A publication's badge flips when a subscriber appears on the far peer,
/// and flips back when it undeclares.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_publication_sees_its_own_subscribers_appear_and_leave() {
    let (a, b) = peer_pair(7485).await;

    let publication = declare_publication(&a, KEY, QosProfile::Transition, None)
        .await
        .expect("declare publication");
    let events = publication.matching_events().await.expect("events");
    assert!(
        !publication.matching_status().await.expect("status"),
        "no subscriber exists yet, anywhere"
    );

    let subscriber = b.declare_subscriber(KEY).await.expect("subscriber");
    let ev = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("matching event within 5s")
        .expect("listener alive");
    assert!(ev, "a matching subscriber must raise the badge");

    subscriber.undeclare().await.expect("undeclare subscriber");
    let ev = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("unmatching event within 5s")
        .expect("listener alive");
    assert!(!ev, "the last subscriber leaving must lower it");
}

/// A repeating query's badge flips when a queryable starts serving the
/// keyexpr it asks on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_repeating_query_sees_a_server_appear() {
    let (a, b) = peer_pair(7486).await;

    let repeating = declare_repeating(&b, "", KEY, Duration::from_secs(5))
        .await
        .expect("declare");
    let events = repeating.matching_events().await.expect("events");

    let _queryable = a
        .declare_queryable(KEY)
        .callback(|_| {})
        .await
        .expect("queryable");
    let ev = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("matching event within 5s")
        .expect("listener alive");
    assert!(ev, "a serving queryable must raise the badge");
    assert!(
        repeating.matching_status().await.expect("status"),
        "status agrees with the event"
    );
}

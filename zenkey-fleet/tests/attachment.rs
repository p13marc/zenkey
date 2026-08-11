//! Attachments are a wire fact and the engine carries them (#117): on the
//! subscribe path (`SampleView`), the fan-in path (`FleetAnswer`), and the
//! fetch ladder (`FetchedValue`) — refcounted like the payload, `None` when
//! the wire carried none (absence is a fact too, never a default).
//!
//! Event-driven: the matching badge proves routability before publishing.
//! Ports 7517-7519 (disjoint from every other test binary).

use std::time::Duration;

use zenkey::qos::QosProfile;
use zenkey_fleet::declare_publication;

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

const KEY: &str = "v1/h-eeeeeeeeeeee/state/demo/health";

/// The Monitor delivers the attachment beside the payload — and a sample
/// without one delivers `None`, not an empty buffer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_watched_sample_carries_its_attachment() {
    let (a, b) = peer_pair(7517).await;

    let monitor = zenkey_fleet::Monitor::start(&b, zenkey_fleet::MonitorSpec::default())
        .await
        .expect("monitor");
    let mut events = monitor.events();
    monitor.watch(KEY).await.expect("watch");

    let publication = declare_publication(&a, KEY, QosProfile::Transition, None)
        .await
        .expect("declare");
    let matching = publication.matching_events().await.expect("events");
    assert!(
        tokio::time::timeout(Duration::from_secs(5), matching.recv())
            .await
            .expect("matching within 5s")
            .expect("listener alive")
    );

    publication
        .send(b"{}".to_vec(), Some(b"meta".to_vec()))
        .await
        .expect("send with attachment");
    publication
        .send(b"{}".to_vec(), None)
        .await
        .expect("send without");

    let mut views = Vec::new();
    while views.len() < 2 {
        let item = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("event within 5s")
            .expect("stream alive");
        if let zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::Sample(s)) = item {
            views.push(s);
        }
    }
    let first = views[0].attachment.as_ref().expect("first carried one");
    assert_eq!(first.to_bytes().as_ref(), b"meta");
    assert!(
        views[1].attachment.is_none(),
        "no attachment on the wire is None, not an empty buffer"
    );
}

/// `fleet_get` answers carry the replier's attachment; a replier that sends
/// none yields `None`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fleet_answer_carries_the_reply_attachment() {
    let (a, b) = peer_pair(7518).await;

    let _queryable = a
        .declare_queryable(KEY)
        .callback(move |query| {
            let with = query.key_expr().as_str().to_string();
            tokio::spawn(async move {
                query
                    .reply(with, b"{\"ok\":true}".to_vec())
                    .attachment(b"who-answered".to_vec())
                    .await
                    .expect("reply");
            });
        })
        .await
        .expect("queryable");

    // Settle: loop the GET until the queryable answers (wait-routable).
    let answers = loop {
        let answers = zenkey_fleet::fleet_get(&b, "", KEY, None, Duration::from_millis(500))
            .await
            .expect("get");
        if !answers.is_empty() {
            break answers;
        }
    };
    let att = answers[0].attachment.as_ref().expect("attachment carried");
    assert_eq!(att.to_bytes().as_ref(), b"who-answered");
}

/// The fetch ladder's window rung carries the attachment too — what the
/// zengui detail pane renders.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fetched_value_carries_the_attachment() {
    let (a, b) = peer_pair(7519).await;

    let publication = declare_publication(&a, KEY, QosProfile::Sampled, None)
        .await
        .expect("declare");
    let matching = publication.matching_events().await.expect("events");

    let fetch = tokio::spawn(async move {
        zenkey_fleet::fetch_value(
            &b,
            KEY,
            zenkey_fleet::FetchSpec {
                // No storage in this fixture: keep the GET rungs short so the
                // window rung (the one under test) opens quickly.
                get_timeout: Duration::from_millis(300),
                window: Duration::from_secs(5),
            },
        )
        .await
    });

    // The fetch's window subscriber raises the badge; then publish into it.
    assert!(
        tokio::time::timeout(Duration::from_secs(5), matching.recv())
            .await
            .expect("matching within 5s")
            .expect("listener alive")
    );
    publication
        .send(b"{\"v\":1}".to_vec(), Some(b"tag".to_vec()))
        .await
        .expect("send");

    let outcome = fetch.await.expect("join").expect("fetch");
    match outcome {
        zenkey_fleet::FetchOutcome::Value(v) => {
            let att = v.attachment.expect("attachment carried");
            assert_eq!(att.to_bytes().as_ref(), b"tag");
        }
        other => panic!("expected a value, got {other:?}"),
    }
}

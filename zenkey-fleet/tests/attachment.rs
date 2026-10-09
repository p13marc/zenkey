//! Attachments are a wire fact and the engine carries them (#117): on the
//! subscribe path (`SampleView`) and the fan-in path (`FleetAnswer`) —
//! refcounted like the payload, `None` when the wire carried none (absence
//! is a fact too, never a default).
//!
//! Event-driven: the matching badge proves routability before publishing.
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::time::Duration;

use zenkey_fleet::WireQos;
use zenkey_fleet::declare_publication;

mod util;
use util::peer_pair;

const KEY: &str = "demo/plant/line-1/health";

/// Axes no default spells, so the view can only have read them off the wire.
const DECLARED: WireQos = WireQos {
    priority: zenoh::qos::Priority::DataHigh,
    congestion: zenoh::qos::CongestionControl::Block,
    reliability: zenoh::qos::Reliability::Reliable,
    express: false,
};

/// The Monitor delivers the attachment beside the payload — and a sample
/// without one delivers `None`, not an empty buffer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_watched_sample_carries_its_attachment() {
    let (a, b) = peer_pair().await;

    let monitor = zenkey_fleet::Monitor::start(&b, zenkey_fleet::MonitorSpec::default())
        .await
        .expect("monitor");
    let mut events = monitor.events();
    monitor.watch(KEY).await.expect("watch");

    let publication = declare_publication(&a, KEY, DECLARED, None)
        .await
        .expect("declare");
    let matching = publication.matching_events().await.expect("events");
    assert!(
        tokio::time::timeout(util::SETTLE, matching.recv())
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
        let item = tokio::time::timeout(util::SETTLE, events.recv())
            .await
            .expect("event within 5s")
            .expect("stream alive");
        if let zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::Sample(s)) = item {
            views.push(s);
        }
    }
    let first = views[0].attachment.as_ref().expect("first carried one");
    assert_eq!(first.to_bytes().as_ref(), b"meta");
    // #120: the wire's actual QoS axes ride the view, as the publication
    // declared them.
    assert_eq!(views[0].wire_qos(), DECLARED);
    assert!(
        views[1].attachment.is_none(),
        "no attachment on the wire is None, not an empty buffer"
    );
}

/// `fleet_get` answers carry the replier's attachment; a replier that sends
/// none yields `None`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fleet_answer_carries_the_reply_attachment() {
    let (a, b) = peer_pair().await;

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
        let answers = zenkey_fleet::fleet_get(
            &b,
            KEY,
            &zenkey_fleet::GetOpts::new(Duration::from_millis(500)),
        )
        .await
        .expect("get");
        if !answers.is_empty() {
            break answers;
        }
    };
    let att = answers[0].attachment.as_ref().expect("attachment carried");
    assert_eq!(att.to_bytes().as_ref(), b"who-answered");
}

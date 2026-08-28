//! The event sources are `Stream`s (#343) — proven against real zenoh,
//! because the point of the change is that a consumer can compose these
//! rather than write a loop, and only a real bus shows they compose.
//!
//! The engine had seven pull APIs and no `Stream` anywhere, so every consumer
//! that wanted one rebuilt the adapter. Four of the seven turned out to be
//! free — zenoh's channel handlers already hand out a `Stream`, so those are
//! projections of it and keep their `&self` receivers, which is what lets a
//! query still be answered while its stream is held (#333).
//!
//! What each test asserts is not "a stream exists" but that the stream
//! carries the *same* honesty the `recv` does: seed drops still arrive as
//! `SeedItem::Dropped`, monitor lag still arrives as `StreamItem::Dropped`
//! and still folds into the monitor's counter.

use futures_util::StreamExt as _;
use zenkey_fleet::{SeedItem, SeedPolicy, StreamItem, seed_subscribe};

mod util;
use util::{peer_pair, timestamping_pair};

/// A responder's queries compose as a stream — and the query it yields is
/// still answerable, because the stream borrows rather than consuming.
///
/// That is the property #333 bought and this must not spend: `answer` takes
/// `&self`, so holding the stream cannot lock the responder out of replying.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_responders_queries_stream_and_stay_answerable() {
    let (a, b) = peer_pair().await;
    let key = "v1/h-cccccccccccc/@rpc/demo/ping";
    let responder = zenkey_fleet::declare_responder(&a, key, b"pong".to_vec(), None, false)
        .await
        .expect("declare");

    let served = tokio::spawn(async move {
        let query = {
            let mut queries = std::pin::pin!(responder.stream());
            queries.next().await.expect("one ask")
        };
        // The stream is dropped; the responder is not, and the query is
        // still in hand — `answer` takes `&self`, which is the property
        // #333 bought and a `&mut self` stream would have spent.
        let view = responder.answer(query).await;
        responder.undeclare().await.expect("undeclare");
        view
    });

    // Settle: loop the GET until the queryable is routable, the house shape.
    let answers = loop {
        let answers = zenkey_fleet::fleet_get(
            &zenkey_fleet::Fleet::new(&b, ""),
            key,
            &zenkey_fleet::GetOpts::new(std::time::Duration::from_millis(500)),
        )
        .await
        .expect("get");
        if !answers.is_empty() {
            break answers;
        }
    };

    match &answers[0].answer {
        zenkey_fleet::Answer::Value(bytes) => assert_eq!(bytes.to_bytes().as_ref(), b"pong"),
        other => panic!("expected a value, got {other:?}"),
    }
    let view = served.await.expect("join");
    assert_eq!(
        view.reply_error, None,
        "the query taken from the stream was answered, not dropped"
    );
}

/// A publication's matching changes compose as a stream, projected to the
/// same `bool` `recv` yields.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn matching_changes_stream_as_bools() {
    let (a, b) = peer_pair().await;
    let key = "v1/h-cccccccccccc/state/demo/health";
    let publication =
        zenkey_fleet::declare_publication(&a, key, zenkey::qos::QosProfile::Transition, None)
            .await
            .expect("declare");
    let events = publication.matching_events().await.expect("events");

    let _sub = b.declare_subscriber(key).await.expect("subscribe");

    let appeared = tokio::time::timeout(util::SETTLE, async {
        let mut changes = std::pin::pin!(events.stream());
        changes.next().await
    })
    .await
    .expect("a matching change within 5s")
    .expect("the listener is alive");
    assert!(appeared, "a subscriber appeared");
}

/// The seeded subscription **is** a stream: seed phase, boundary and live
/// phase are one sequence, in that order — the ordering the type exists for.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_seeded_subscription_streams_seed_then_boundary_then_live() {
    use zenoh_ext::AdvancedPublisherBuilderExt;

    let (a, b) = timestamping_pair().await;
    let key = "v1/h-cccccccccccc/state/demo/health";
    let publisher = a
        .declare_publisher(key)
        .cache(zenoh_ext::CacheConfig::default().max_samples(4))
        .await
        .expect("cached publisher");
    publisher.put("seeded").await.expect("put");
    tokio::time::sleep(util::SETTLE / 10).await;

    let sub = seed_subscribe(&b, key, SeedPolicy::default())
        .await
        .expect("seed subscribe");

    let mut items = std::pin::pin!(sub);
    let mut seeded = Vec::new();
    let coverage = tokio::time::timeout(util::SETTLE, async {
        loop {
            match items.next().await.expect("stream alive") {
                SeedItem::Sample(v) => {
                    seeded.push(String::from_utf8_lossy(&v.payload.to_bytes()).to_string());
                }
                SeedItem::Dropped(n) => panic!("this fixture never outruns the channel ({n})"),
                SeedItem::SeedComplete(c) => break c,
            }
        }
    })
    .await
    .expect("the boundary within 5s");

    assert_eq!(
        seeded,
        vec!["seeded"],
        "the seed arrived before the boundary"
    );
    assert!(
        coverage.history_replies.is_some() || coverage.storage_replies.is_some(),
        "the seed was asked for: {coverage:?}"
    );

    // Live samples keep flowing through the same stream after the boundary.
    publisher.put("live").await.expect("put");
    let live = tokio::time::timeout(util::SETTLE, async {
        loop {
            if let SeedItem::Sample(v) = items.next().await.expect("stream alive") {
                return String::from_utf8_lossy(&v.payload.to_bytes()).to_string();
            }
        }
    })
    .await
    .expect("a live sample within 5s");
    assert_eq!(live, "live");
}

/// The monitor's stream carries lag as **data**, not as an error, and still
/// folds it into the monitor's own counter.
///
/// This is the assertion that rules out a generic broadcast-to-stream
/// adapter: those surface lag as `Err(Lagged(n))` and never touch the
/// counter, which would make a dropped sample invisible in exactly the report
/// that exists to admit it (O6).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_monitors_stream_reports_lag_as_data_and_counts_it() {
    let (a, b) = peer_pair().await;
    let key = "v1/h-cccccccccccc/state/demo/health";

    let monitor = zenkey_fleet::Monitor::start(&b, zenkey_fleet::MonitorSpec::default())
        .await
        .expect("monitor");
    // Taken before the flood, so the ring is what overruns it.
    let slow = monitor.events();
    let monitor = monitor.watching([key]).await.expect("watch");

    let publication =
        zenkey_fleet::declare_publication(&a, key, zenkey::qos::QosProfile::Transition, None)
            .await
            .expect("declare");
    let matching = publication.matching_events().await.expect("events");
    assert!(
        tokio::time::timeout(util::SETTLE, matching.recv())
            .await
            .expect("matching within 5s")
            .expect("listener alive")
    );

    // Outrun the bounded broadcast without draining it.
    for i in 0..4096u32 {
        publication
            .send(i.to_string().into_bytes(), None)
            .await
            .expect("send");
    }
    tokio::time::sleep(util::SETTLE / 10).await;

    let mut items = std::pin::pin!(slow.into_stream());
    let dropped = tokio::time::timeout(util::SETTLE, async {
        loop {
            match items.next().await.expect("stream alive") {
                StreamItem::Dropped(n) => return n,
                StreamItem::Event(_) => continue,
            }
        }
    })
    .await
    .expect("a drop report within 5s");

    assert!(dropped > 0, "the flood outran the ring");
    assert!(
        monitor.core().dropped() >= dropped,
        "the stream folded its lag into the monitor's counter: \
         monitor says {}, stream reported {dropped}",
        monitor.core().dropped()
    );
    monitor.shutdown().await.expect("shutdown");
}

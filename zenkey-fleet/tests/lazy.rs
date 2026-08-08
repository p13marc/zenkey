//! The lazy-observation contract (issue #84), proven against real zenoh —
//! self-contained: two in-process peers, explicit endpoints, no scouting, no
//! external router.
//!
//! The proofs deliberately run at the *peer's routing layer* via matching
//! status, not by asserting an absence of samples: "no subscriber declared
//! anywhere ⇒ zenoh sends nothing" is the network-level fact laziness rests
//! on, and `MatchingListener` observes it event-driven, without sleeps.

use std::time::Duration;

use zenkey_fleet::{FetchOutcome, Monitor, MonitorSpec, ValueSource};

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

/// Zero data-plane subscriptions before the first watch; watch delivers;
/// unwatch provably undeclares (the publisher's matching status flips back).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_and_unwatch_are_visible_at_the_routing_layer() {
    let (a, b) = peer_pair(7461).await;

    let publisher = a
        .declare_publisher("demo/lazy/key")
        .await
        .expect("declare publisher");
    let matching = publisher
        .matching_listener()
        .await
        .expect("matching listener");

    // Lazy start: empty selectors = no data-plane subscribers at all.
    let monitor = Monitor::start(
        &b,
        MonitorSpec {
            selectors: vec![],
            ..Default::default()
        },
    )
    .await
    .expect("lazy monitor");
    assert!(
        !publisher.matching_status().await.unwrap().matching(),
        "before any watch, no subscriber may exist anywhere"
    );
    assert_eq!(monitor.core().with_stats(|s| s.len()), 0);

    // Watch: the publisher sees a subscriber appear (event-driven, no sleep).
    let id = monitor.watch("demo/lazy/**").await.expect("watch");
    let ev = tokio::time::timeout(Duration::from_secs(5), matching.recv_async())
        .await
        .expect("matching event within 5s")
        .expect("listener alive");
    assert!(ev.matching(), "watch must declare a real subscriber");

    // Traffic lands in stats.
    publisher.put("hello").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if monitor.core().with_stats(|s| s.len()) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("sample should land");

    // Unwatch: undeclare is acknowledged, the publisher's matching flips
    // back, and the stats retire under the O6 counter.
    monitor.unwatch(id).await.expect("unwatch");
    let ev = tokio::time::timeout(Duration::from_secs(5), matching.recv_async())
        .await
        .expect("unmatching event within 5s")
        .expect("listener alive");
    assert!(!ev.matching(), "unwatch must undeclare, provably");
    assert_eq!(monitor.core().with_stats(|s| s.len()), 0);
    assert_eq!(
        monitor.core().keys_unwatched(),
        1,
        "retirement is counted, never silent (O6)"
    );
}

/// The fetch ladder reports its source: a queryable at the concrete key is
/// `storage`; a live publisher only is `window`; nothing is an attributed
/// `none`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_value_reports_its_source() {
    let (a, b) = peer_pair(7462).await;

    // Rung 1: a queryable standing at the concrete key (storage-shaped).
    let _queryable = a
        .declare_queryable("demo/fetch/stored")
        .callback(|query| {
            let q = query.clone();
            tokio::spawn(async move {
                q.reply("demo/fetch/stored", "stored-value").await.ok();
            });
        })
        .await
        .expect("queryable");
    // Give the declaration a moment to propagate to the peer.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let out = zenkey_fleet::fetch_value(&b, "demo/fetch/stored", Default::default())
        .await
        .expect("fetch");
    match out {
        FetchOutcome::Value(v) => {
            assert_eq!(v.source, ValueSource::Storage);
            assert_eq!(v.payload.to_bytes().as_ref(), b"stored-value");
        }
        other => panic!("expected a stored value, got {other:?}"),
    }

    // Rung 3: only a live publisher — the window catches one.
    let publisher = a
        .declare_publisher("demo/fetch/live")
        .await
        .expect("publisher");
    let pump = tokio::spawn(async move {
        loop {
            publisher.put("live-value").await.ok();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    let out = zenkey_fleet::fetch_value(&b, "demo/fetch/live", Default::default())
        .await
        .expect("fetch");
    pump.abort();
    match out {
        FetchOutcome::Value(v) => assert_eq!(v.source, ValueSource::Window),
        other => panic!("expected a windowed value, got {other:?}"),
    }

    // Nothing at all: an attributed non-verdict, fast-bounded for the test.
    let spec = zenkey_fleet::FetchSpec {
        get_timeout: Duration::from_millis(400),
        window: Duration::from_millis(300),
    };
    let out = zenkey_fleet::fetch_value(&b, "demo/fetch/absent", spec)
        .await
        .expect("fetch");
    match out {
        FetchOutcome::None { attempted } => {
            assert_eq!(attempted, ["get", "@adv cache", "subscribe window"]);
        }
        other => panic!("expected None, got {other:?}"),
    }
}

/// The `@adv` cache rung: an AdvancedPublisher with a cache answers the
/// `<key>/@adv/**` GET even with no storage and no live traffic.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_value_reaches_the_advanced_cache() {
    use zenoh_ext::AdvancedPublisherBuilderExt;

    // An AdvancedPublisher requires session timestamping (its sequencing is
    // timestamp-based) — the publisher side gets a bespoke config here; the
    // fetching side stays the plain explorer session.
    let mut cfg = zenoh::Config::default();
    cfg.insert_json5("scouting/multicast/enabled", "false").ok();
    cfg.insert_json5("timestamping/enabled", "true").ok();
    cfg.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:7463\"]")
        .ok();
    let a = zenoh::open(cfg).await.expect("timestamping session");
    let b = zenkey_fleet::session::open(&["tcp/127.0.0.1:7463".to_string()], &[], false)
        .await
        .expect("connector session");
    let publisher = a
        .declare_publisher("demo/fetch/cached")
        .cache(zenoh_ext::CacheConfig::default().max_samples(1))
        .await
        .expect("advanced publisher");
    publisher.put("cached-value").await.expect("cached put");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let spec = zenkey_fleet::FetchSpec {
        get_timeout: Duration::from_millis(800),
        window: Duration::from_millis(300),
    };
    let out = zenkey_fleet::fetch_value(&b, "demo/fetch/cached", spec)
        .await
        .expect("fetch");
    match out {
        FetchOutcome::Value(v) => {
            assert_eq!(v.source, ValueSource::Cache, "the @adv rung answered");
            assert_eq!(v.payload.to_bytes().as_ref(), b"cached-value");
        }
        other => panic!("expected the cache to answer, got {other:?}"),
    }
}

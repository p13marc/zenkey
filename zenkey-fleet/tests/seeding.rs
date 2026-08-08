//! RFC 04 §3.2's seed discipline, proven against real zenoh (issue #42) —
//! self-contained two-peer sessions, no router.

use std::time::Duration;

use zenkey_fleet::{SeedItem, SeedPolicy, seed_subscribe};
use zenoh_ext::AdvancedPublisherBuilderExt;

async fn timestamping_listener(port: u16) -> zenoh::Session {
    let mut cfg = zenoh::Config::default();
    cfg.insert_json5("scouting/multicast/enabled", "false").ok();
    cfg.insert_json5("timestamping/enabled", "true").ok();
    cfg.insert_json5("listen/endpoints", &format!("[\"tcp/127.0.0.1:{port}\"]"))
        .ok();
    zenoh::open(cfg).await.expect("publisher session")
}

/// Drain a seeded subscriber until the boundary; returns (payloads, coverage).
async fn drain_seed(
    sub: &mut zenkey_fleet::SeededSubscriber,
) -> (Vec<String>, zenkey_fleet::SeedCoverage) {
    let mut values = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(5), sub.recv())
            .await
            .expect("seed boundary within 5s")
            .expect("stream alive")
        {
            SeedItem::Sample(v) => {
                values.push(String::from_utf8_lossy(&v.payload.to_bytes()).to_string())
            }
            SeedItem::SeedComplete(c) => return (values, c),
        }
    }
}

/// The history seed reaches a live publisher's cache — and the boundary
/// arrives strictly AFTER the seed (the race this module exists to close:
/// a boundary that outruns the cache reply turns "loading" into "empty").
/// Live samples keep flowing after the boundary.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_seed_lands_before_the_boundary() {
    let a = timestamping_listener(7471).await;
    let b = zenkey_fleet::session::open(&["tcp/127.0.0.1:7471".to_string()], &[], false)
        .await
        .expect("consumer session");

    let publisher = a
        .declare_publisher("seedtest/state/health")
        .cache(zenoh_ext::CacheConfig::default().max_samples(1))
        .await
        .expect("advanced publisher");
    publisher.put("v1").await.expect("cached put");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut sub = seed_subscribe(
        &b,
        "seedtest/state/**",
        SeedPolicy {
            timeout: Duration::from_millis(800),
            ..SeedPolicy::default()
        },
    )
    .await
    .expect("seed subscribe");

    let (seen, coverage) = drain_seed(&mut sub).await;
    assert_eq!(seen, ["v1"], "the cached value seeds — before the boundary");
    assert_eq!(coverage.history_replies, Some(1), "the cache answered once");
    assert_eq!(
        coverage.storage_replies,
        Some(0),
        "no storage on this bus — ran and found nothing, an observation"
    );

    // Live after the boundary.
    publisher.put("v2").await.expect("live put");
    let item = tokio::time::timeout(Duration::from_secs(5), sub.recv())
        .await
        .expect("live within 5s")
        .expect("stream alive");
    match item {
        SeedItem::Sample(v) => assert_eq!(v.payload.to_bytes().as_ref(), b"v2"),
        other => panic!("expected the live sample, got {other:?}"),
    }
}

/// The LWW merge suppresses a stale storage seed: a storage-shaped queryable
/// whose (unstamped) reply arrives after the stamped cache value must not
/// regress the key — and the suppression is counted, never silent (O6).
/// The storage reply is delayed so the order is deterministic: stamped
/// state first, stale echo second.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_storage_seed_cannot_regress_a_key() {
    let a = timestamping_listener(7472).await;
    let b = zenkey_fleet::session::open(&["tcp/127.0.0.1:7472".to_string()], &[], false)
        .await
        .expect("consumer session");

    // The publisher's cache holds the CURRENT value, stamped now.
    let publisher = a
        .declare_publisher("staletest/state/doc")
        .cache(zenoh_ext::CacheConfig::default().max_samples(1))
        .await
        .expect("advanced publisher");
    publisher.put("current").await.expect("put");

    // A "storage" that answers late and UNstamped — by the time it replies,
    // the stamped cache value has already seeded the key; the merge must
    // refuse the regression (stamped state beats an unstamped echo).
    let _storage = a
        .declare_queryable("staletest/state/doc")
        .callback(move |query| {
            let q = query.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                q.reply("staletest/state/doc", "stale-from-storage")
                    .await
                    .ok();
            });
        })
        .await
        .expect("queryable");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut sub = seed_subscribe(
        &b,
        "staletest/state/**",
        SeedPolicy {
            timeout: Duration::from_millis(800),
            ..SeedPolicy::default()
        },
    )
    .await
    .expect("seed subscribe");

    let (values, coverage) = drain_seed(&mut sub).await;
    assert_eq!(
        values,
        ["current"],
        "the stamped cache value seeds once; the late unstamped echo never surfaces"
    );
    assert_eq!(coverage.storage_replies, Some(1), "the storage DID answer");
    assert!(
        coverage.superseded >= 1,
        "…and its suppression is counted, not silent (O6)"
    );
}

/// The gap rule: a sample published immediately after `seed_subscribe`
/// returns — racing the seed GETs — arrives exactly once. This is the
/// transition GET-then-subscribe silently drops.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_transition_in_the_seed_window_lands_exactly_once() {
    let a = timestamping_listener(7473).await;
    let b = zenkey_fleet::session::open(&["tcp/127.0.0.1:7473".to_string()], &[], false)
        .await
        .expect("consumer session");
    let publisher = a
        .declare_publisher("gaptest/state/flag")
        .await
        .expect("publisher");
    let matching = publisher
        .matching_listener()
        .await
        .expect("matching listener");
    // A storage sim that holds the query open (replying nothing) — it keeps
    // the seed window from closing before the in-window put below.
    let _slow_storage = a
        .declare_queryable("gaptest/state/**")
        .callback(|query| {
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(600)).await;
                drop(query);
            });
        })
        .await
        .expect("queryable");

    let mut sub = seed_subscribe(
        &b,
        "gaptest/state/**",
        SeedPolicy {
            history: false, // plain publisher; the subscriber-first order is the point
            timeout: Duration::from_millis(800),
            ..SeedPolicy::default()
        },
    )
    .await
    .expect("seed subscribe");

    // Wait for the subscriber's interest to reach the publishing peer (a
    // network-propagation concern, not part of the seed contract), then
    // publish inside the seed window.
    let ev = tokio::time::timeout(Duration::from_secs(5), matching.recv_async())
        .await
        .expect("matching event within 5s")
        .expect("listener alive");
    assert!(ev.matching(), "the seed subscriber is a real subscriber");
    publisher.put("flank").await.expect("put");

    let mut before_boundary = 0;
    let mut after_boundary = 0;
    let mut done = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        match tokio::time::timeout_at(deadline, sub.recv()).await {
            Ok(Some(SeedItem::Sample(v))) => {
                assert_eq!(v.payload.to_bytes().as_ref(), b"flank");
                if done {
                    after_boundary += 1;
                } else {
                    before_boundary += 1;
                }
            }
            Ok(Some(SeedItem::SeedComplete(c))) => {
                assert_eq!(c.history_replies, None, "history was opted out");
                done = true;
            }
            Ok(None) | Err(_) => break,
        }
    }
    assert!(done, "the seed boundary must arrive");
    assert_eq!(
        (before_boundary, after_boundary),
        (1, 0),
        "the in-window transition lands exactly once, inside the seed phase"
    );
}

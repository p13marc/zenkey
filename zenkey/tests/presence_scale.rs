//! `spec/scenarios/presence.md` §4 (spec §8.1), in a binary of its own:
//! ten thousand tokens load the router, and the other presence scenarios
//! should not run beside that.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{client, router};
use zenkey::presence;

/// §4: a liveliness GET on a session holding a liveliness subscriber
/// completes with every token, on an unbounded handler.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_reading_presence_at_scale() {
    const N: usize = 10_000;
    let (_r1, ep) = router(None).await;
    let holder = client(&ep).await;
    let reader = client(&ep).await;
    let mut tokens = Vec::with_capacity(N);
    for i in 0..N {
        tokens.push(
            holder
                .liveliness()
                .declare_token(format!("zk2/p4/svc{i}/@zk/instance/{i:016x}"))
                .await
                .unwrap(),
        );
    }
    let count = Arc::new(Mutex::new(0usize));
    let c = Arc::clone(&count);
    let _sub = reader
        .liveliness()
        .declare_subscriber("zk2/p4/*/@zk/**")
        .callback(move |_| *c.lock().unwrap() += 1)
        .await
        .unwrap();
    // Ten thousand client declarations take tens of seconds to settle on
    // a debug-built in-process router, so propagation gets its own budget.
    // What the rule is about is the last GET: complete, with every token.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let got = presence::liveliness_keys(&reader, "zk2/p4/*/@zk/**", Duration::from_secs(30))
            .await
            .unwrap();
        if got.len() == N {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{} of {N} tokens after 180 s",
            got.len()
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(*count.lock().unwrap() <= N);
}

/// A client whose runtime waits `interests_ms` for its router's final reply
/// to a current interest (`routing/interests/timeout`, zenoh's 10 s by
/// default).
async fn client_waiting(endpoint: &str, interests_ms: u64) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .unwrap();
    c.insert_json5("routing/interests/timeout", &interests_ms.to_string())
        .unwrap();
    zenoh::open(c).await.unwrap()
}

/// #851 (WB), measured, and slow (about two minutes in a debug build), so
/// run by hand (`--ignored`): the barrier beside ten thousand tokens, asked
/// with a selector that matches no token on a watch session at once after
/// its subscriber, as `Monitor::start` asks it; with history and without,
/// under zenoh's own `routing/interests/timeout` (10 s) and a longer one;
/// then a read of every token, under each.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "a measurement: about two minutes"]
async fn wb_a_barrier_beside_ten_thousand_tokens_measured() {
    const N: usize = 10_000;
    let (_r1, ep) = router(None).await;
    let holders: Vec<zenoh::Session> = {
        let mut v = Vec::new();
        for _ in 0..4 {
            v.push(client(&ep).await);
        }
        v
    };
    let reader = client(&ep).await;
    let started = std::time::Instant::now();
    let mut tokens = Vec::with_capacity(N);
    for i in 0..N {
        tokens.push(
            holders[i % holders.len()]
                .liveliness()
                .declare_token(format!("zk2/wb/svc{i}/@zk/instance/{i:016x}"))
                .await
                .unwrap(),
        );
    }
    let declared = started.elapsed();
    loop {
        let r =
            presence::liveliness_read(&reader, "zk2/wb/*/@zk/instance/*", Duration::from_secs(30))
                .await
                .unwrap();
        if r.complete && r.keys.len() == N {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    eprintln!(
        "setup: declared in {declared:?}, read whole after {:?}",
        started.elapsed()
    );
    for (history, interests_ms) in [(true, 10_000), (true, 120_000), (false, 120_000)] {
        let watch = client_waiting(&ep, interests_ms).await;
        let count = Arc::new(Mutex::new((0usize, None::<std::time::Instant>)));
        let c = Arc::clone(&count);
        let t = std::time::Instant::now();
        let _sub = presence::declare_subscriber(&watch, "zk2/*/*/@zk/instance/*")
            .history(history)
            .callback(move |_| {
                let mut c = c.lock().unwrap();
                c.0 += 1;
                c.1 = Some(std::time::Instant::now());
            })
            .await
            .unwrap();
        let first = presence::settle(&watch, "zk2/*/*/@zk/settle", Duration::from_secs(110))
            .await
            .unwrap();
        let first_took = t.elapsed();
        let heard_then = count.lock().unwrap().0;
        let t2 = std::time::Instant::now();
        let again = presence::settle(&watch, "zk2/*/*/@zk/settle", Duration::from_secs(110))
            .await
            .unwrap();
        let again_took = t2.elapsed();
        let (heard, last) = *count.lock().unwrap();
        eprintln!(
            "beside {N}, history {history}, interests timeout {interests_ms} ms: the barrier \
             {first} {first_took:?} after the subscriber, {heard_then} heard by then; again \
             {again} in {again_took:?}; heard {heard}, the last {:?} after the subscriber",
            last.map(|l| l - t)
        );
        let _ = watch.close().await;
    }
    for interests_ms in [120_000, 10_000] {
        let fresh = client_waiting(&ep, interests_ms).await;
        let t = std::time::Instant::now();
        let read =
            presence::liveliness_read(&fresh, "zk2/*/*/@zk/instance/*", Duration::from_secs(110))
                .await
                .unwrap();
        eprintln!(
            "a read of every token on a fresh session, interests timeout {interests_ms} ms, a \
             110 s GET: complete {} with {} tokens, errors {:?}, in {:?}",
            read.complete,
            read.keys.len(),
            read.errors,
            t.elapsed()
        );
    }
}

//! A tool's watch session beside a large population in a namespace (#875,
//! core §8.1, 0.42), in a binary of its own: the crowd's tokens load the
//! router, and nothing else should run beside them.
//!
//! Before #875 a watch session in a namespace kept history on its
//! subscribers, since zenoh 1.10.1 never delivers a withdrawal of an
//! earlier token to a namespaced subscriber without it (#872); and its round
//! trip waited for that history, about 23 s beside 10,000 tokens in a debug
//! build (`zenkey/tests/presence_scale.rs`'s ignored
//! `wb_a_barrier_beside_ten_thousand_tokens_measured`), past every tool's
//! 5 s `--timeout`. The tools' watch session is now in no namespace, on the
//! wire keys, without history (`zenkey_fleet::open_watch`), and its round
//! trip at its start carries nothing at any population. After a move it
//! waits for the population (core §8.1, 0.44, F-239): `watch_reopen.rs`.

mod util;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use util::zk2::{eventually, router};
use zenkey_fleet::{
    AddressStatus, LivePresence, Monitor, MonitorSpec, PresenceFeed, PresenceScope, SeedOutcome,
};

/// The crowd: ten thousand instance tokens, `presence.md` §4's population.
const CROWD: usize = 10_000;

/// The bound a round trip beside the crowd keeps: one round trip, with a
/// debug build's and a small CI runner's room to spare. With history it
/// took 23 s here.
const ONE_ROUND_TRIP: Duration = Duration::from_secs(1);

/// §10 step 4 in a namespace, through the tools' own wiring (#875): beside
/// 10,000 instance tokens declared in `ns` before it, a watch session for
/// `ns` opened by `open_watch` makes its round trip within one round trip,
/// a crowd token withdrawn after it is heard going, a presence monitor on
/// another stands at its start, and the feed seeded beside it is complete.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_barrier_beside_ten_thousand_tokens_in_a_namespace_is_one_round_trip() {
    let (_router, ep) = router(None).await;
    let endpoints = std::slice::from_ref(&ep);
    let open_ns = || async {
        zenkey_fleet::open_in_namespace("ns", None, endpoints, &[], Some(false))
            .await
            .expect("a session in ns")
    };
    let holders = [
        open_ns().await,
        open_ns().await,
        open_ns().await,
        open_ns().await,
    ];
    // A tool's session: its runtime waits 120 s for its routers, so a read
    // of the whole crowd can be complete (core §8.1, 0.41).
    let tool = open_ns().await;
    let started = Instant::now();
    let mut crowd = Vec::with_capacity(CROWD);
    for i in 0..CROWD {
        crowd.push(
            holders[i % holders.len()]
                .liveliness()
                .declare_token(format!("zk2/crowd/svc{i}/@zk/instance/{:016x}", i + 1))
                .await
                .expect("a token"),
        );
    }
    // The router holds the crowd once it holds each holder's last token:
    // one holder's declarations reach it in order. A read of each, not of
    // the crowd, which takes tens of seconds on a debug-built router.
    // Ten thousand client declarations take tens of seconds to settle on a
    // debug-built in-process router, so the setup gets a budget of its own,
    // as `zenkey/tests/presence_scale.rs`'s §4 does.
    let declared = started.elapsed();
    let deadline = Instant::now() + Duration::from_secs(180);
    for last in CROWD - holders.len()..CROWD {
        let key = format!("zk2/crowd/svc{last}/@zk/instance/{:016x}", last + 1);
        while !zenkey_fleet::liveliness_read(&tool, &key, Duration::from_secs(5))
            .await
            .is_ok_and(|r| r.complete && r.keys == [key.clone()])
        {
            assert!(Instant::now() < deadline, "the router never held {key}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    eprintln!(
        "#875: the crowd of {CROWD} declared in {declared:?}, held by the router after {:?}",
        started.elapsed()
    );

    let open_watch = || async {
        zenkey_fleet::open_watch("ns", None, endpoints, &[], Some(false))
            .await
            .expect("a watch session for ns")
    };

    // The round trip itself, after a subscriber to every instance token.
    let watch = open_watch().await;
    let heard: Arc<Mutex<Vec<zenkey_fleet::TokenHeard>>> = Arc::default();
    let h = Arc::clone(&heard);
    let _sub = watch
        .subscribe(
            "the crowd's instance tokens",
            "zk2/*/*/@zk/instance/*",
            move |t| h.lock().unwrap().push(t),
        )
        .await
        .expect("a subscriber");
    let barrier = watch
        .barrier("zk2/*/*/@zk/instance/*", Duration::from_secs(5))
        .await
        .expect("a barrier");
    assert_eq!(barrier.key(), "ns/zk2/*/*/@zk/settle");
    let t = Instant::now();
    let stood = barrier.establish().await;
    let took = t.elapsed();
    eprintln!("#875: the round trip beside {CROWD} in ns took {took:?}, standing {stood}");
    assert!(stood, "the round trip stands");
    assert!(
        took < ONE_ROUND_TRIP,
        "one round trip beside {CROWD} tokens in a namespace: {took:?}"
    );
    assert!(heard.lock().unwrap().is_empty(), "no history");

    // A token of the crowd, declared before the subscriber and withdrawn
    // after the round trip, is heard going, base-relative.
    let (gone, token) = (
        "zk2/crowd/svc0/@zk/instance/0000000000000001",
        crowd.remove(0),
    );
    token.undeclare().await.expect("undeclare");
    eventually("the withdrawal heard", || async {
        heard
            .lock()
            .unwrap()
            .iter()
            .any(|t| t.key == gone && !t.held)
    })
    .await;
    assert_eq!(watch.outside(), 0);

    // The tools' reader on the same wiring: a monitor on a watch session of
    // its own stands at its start, and the feed beside it seeds complete.
    let watch = open_watch().await;
    let spec = MonitorSpec {
        stats_tick: Duration::from_secs(3600),
        ..MonitorSpec::default()
    }
    .with_presence(&PresenceScope::all());
    let t = Instant::now();
    let monitor = Monitor::start_watch(&watch, spec).await.expect("a monitor");
    let started = t.elapsed();
    eprintln!("#875: a presence monitor beside {CROWD} in ns started in {started:?}");
    assert_eq!(monitor.settled(), Some(true), "its round trip stood");
    // Its seed reads one address of the crowd: its subscribers watch all
    // of it, which is what the round trip stood beside.
    let svc1 = util::zk2::addr("crowd/svc1");
    let mut live = LivePresence::new(PresenceScope::service(&svc1));
    live.track(&svc1, std::time::SystemTime::now());
    let (feed, _) = PresenceFeed::open(&tool, &monitor, live, Duration::from_secs(60))
        .await
        .expect("the feed opens");
    assert_eq!(*feed.last_seed(), SeedOutcome::Complete);
    assert_eq!(feed.presence().status(&svc1), AddressStatus::Up);
    drop(feed);
    monitor.shutdown().await.expect("shutdown");
}

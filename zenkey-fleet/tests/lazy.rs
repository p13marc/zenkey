//! The lazy-observation contract (issue #84), proven against real zenoh —
//! self-contained: two in-process peers, explicit endpoints, no scouting, no
//! external router.
//!
//! The proofs deliberately run at the *peer's routing layer* via matching
//! status, not by asserting an absence of samples: "no subscriber declared
//! anywhere ⇒ zenoh sends nothing" is the network-level fact laziness rests
//! on, and `MatchingListener` observes it event-driven, without sleeps.

use std::time::Duration;

use zenkey_fleet::{Monitor, MonitorSpec};

mod util;
use util::peer_pair;

/// Zero data-plane subscriptions before the first watch; watch delivers;
/// unwatch provably undeclares (the publisher's matching status flips back).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_and_unwatch_are_visible_at_the_routing_layer() {
    let (a, b) = peer_pair().await;

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
    let ev = tokio::time::timeout(util::SETTLE, matching.recv_async())
        .await
        .expect("matching event within 5s")
        .expect("listener alive");
    assert!(ev.matching(), "watch must declare a real subscriber");

    // Traffic lands in stats.
    publisher.put("hello").await.unwrap();
    tokio::time::timeout(util::SETTLE, async {
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
    let ev = tokio::time::timeout(util::SETTLE, matching.recv_async())
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

/// `shutdown` is `unwatch` for the whole monitor: **every** watch undeclares
/// and is waited for.
///
/// Dropping the monitor only aborts its tasks and lets the subscribers
/// undeclare in the background — the race `unwatch`'s doc disavows, and the
/// one a frontend that re-scopes by rebuilding its monitor was running every
/// time.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_undeclares_every_watch() {
    let (a, b) = peer_pair().await;

    let publisher = a
        .declare_publisher("demo/down/key")
        .await
        .expect("declare publisher");
    let matching = publisher
        .matching_listener()
        .await
        .expect("matching listener");

    let monitor = Monitor::start(
        &b,
        MonitorSpec {
            selectors: vec![],
            ..Default::default()
        },
    )
    .await
    .expect("lazy monitor");
    // Two watches, both covering the publisher: the badge falls only when
    // the last of them is gone, so this proves the drain, not one undeclare.
    monitor.watch("demo/down/**").await.expect("watch");
    monitor.watch("demo/**").await.expect("second watch");
    let ev = tokio::time::timeout(util::SETTLE, matching.recv_async())
        .await
        .expect("matching event within 5s")
        .expect("listener alive");
    assert!(ev.matching(), "the watches declared real subscribers");

    monitor.shutdown().await.expect("acknowledged teardown");
    let ev = tokio::time::timeout(util::SETTLE, matching.recv_async())
        .await
        .expect("unmatching event within 5s")
        .expect("listener alive");
    assert!(
        !ev.matching(),
        "shutdown must undeclare every watch, provably"
    );
}

/// `watching` is how a window declares what it observes, and a declaration
/// that fails takes the monitor with it (#336).
///
/// Seven judge windows opened a monitor, took its event stream, then declared
/// their watches with a `?` — three of them on `**`. Any failure there
/// returned with the monitor's liveliness and tick tasks running and its
/// subscribers left to `Drop`: the unacknowledged teardown `shutdown` exists
/// to refuse, on the one path nobody exercises. The error the caller wanted is
/// still the error it gets; what changed is what is left behind.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_watch_takes_down_the_ones_that_came_up() {
    let (a, b) = peer_pair().await;

    let publisher = a
        .declare_publisher("demo/half/key")
        .await
        .expect("declare publisher");
    let matching = publisher
        .matching_listener()
        .await
        .expect("matching listener");

    let monitor = Monitor::start(&b, MonitorSpec::default())
        .await
        .expect("lazy monitor");
    // The stream is taken before anything is declared — the ordering these
    // windows need, and the reason the watches cannot simply move into the
    // spec.
    let _events = monitor.events();

    // The window's first selector comes up …
    let monitor = monitor
        .watching(["demo/half/**"])
        .await
        .expect("the first selector declares");
    let ev = tokio::time::timeout(util::SETTLE, matching.recv_async())
        .await
        .expect("matching event within 5s")
        .expect("listener alive");
    assert!(ev.matching(), "a real subscriber is up");

    // … and the one that fails takes it down on the way out, rather than
    // handing the caller an error and a live subscriber.
    let err = monitor
        .watching(["demo//empty-chunk"])
        .await
        .expect_err("an empty chunk is not a key expression")
        .to_string();
    assert!(err.contains("subscribe"), "{err}");
    let ev = tokio::time::timeout(util::SETTLE, matching.recv_async())
        .await
        .expect("unmatching event within 5s")
        .expect("listener alive");
    assert!(!ev.matching(), "a failed window leaves nothing declared");
}

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

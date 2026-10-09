//! Spec §8.1 in the fleet's own presence reads (#612, FJ3): a liveliness
//! GET on a session that already holds a liveliness subscriber runs on an
//! unbounded handler.
//!
//! zenoh's default 256-slot handler hangs such a GET at every size measured
//! from 996 tokens (zenoh#2678, spike S2), and the fleet's sessions do hold
//! one: a [`zenkey_fleet::Monitor`] can watch tokens with a history
//! subscriber. v1's `roster`, `node_info` and `discover_bases` each ran a
//! liveliness GET on zenoh's default handler until this suite, and against
//! that code this test hung: the deadlock blocks the runtime's own threads,
//! so not even a `tokio::time::timeout` around the read fires. Every
//! liveliness GET now goes through [`zenkey_fleet::liveliness_read`], which
//! this suite drives directly.
//!
//! So the body runs on a thread and a runtime of its own, and the test
//! thread waits for it with a deadline: a regression fails here, by name,
//! instead of hanging the suite. A binary of its own, because the tokens
//! load the router.

use std::time::Duration;

mod util;
use util::zk2::{client, router};
use zenkey_fleet::{Monitor, MonitorSpec};

/// Past zenoh#2678's threshold (996 tokens in spike S2), with margin.
const N: usize = 1_200;

/// Every read below, on a session whose monitor watches the same tokens,
/// returns with every token instead of hanging.
#[test]
fn presence_reads_complete_beside_a_liveliness_subscriber() {
    let (done, wait) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(reads_beside_a_subscriber());
        let _ = done.send(());
    });
    // Propagation of the tokens has its own 120 s budget inside the body;
    // past this, the body is hung, not slow.
    match wait.recv_timeout(Duration::from_secs(300)) {
        Ok(()) => {}
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!(
            "a presence read hung on a session holding a liveliness subscriber \
             (spec §8.1, zenoh#2678)"
        ),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            panic!("the body panicked (its message is above)")
        }
    }
}

async fn reads_beside_a_subscriber() {
    let (_router, ep) = router(None).await;
    let holder = client(&ep).await;
    let reader = client(&ep).await;
    let mut tokens = Vec::with_capacity(N);
    for i in 0..N {
        tokens.push(
            holder
                .liveliness()
                .declare_token(format!("demo/held/p{i:04}/alive"))
                .await
                .expect("token"),
        );
    }

    // A liveliness watch: a subscriber with history, drained as its samples
    // arrive (spec §8.1's "the subscribers bind too").
    let monitor = Monitor::start(
        &reader,
        MonitorSpec {
            liveliness: vec!["demo/held/*/alive".to_owned()],
            ..MonitorSpec::default()
        },
    )
    .await
    .expect("monitor");

    let get = Duration::from_secs(5);

    // The tokens reach the router before the property is asked about.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        let read = zenkey_fleet::liveliness_read(&reader, "demo/held/*/alive", get)
            .await
            .expect("a liveliness read");
        let seen = read.keys.len();
        if seen == N {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{seen} of {N} tokens after 120 s"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // A read across every prefix, the shape `namespace list` asks
    // (`**/zk2/…`), through the same chokepoint: every token, and complete.
    let read = zenkey_fleet::liveliness_read(&reader, "**/held/*/alive", get)
        .await
        .expect("a liveliness read");
    assert_eq!(read.keys.len(), N);
    assert!(read.complete, "a GET that returned before its timeout");
    let none = zenkey_fleet::namespace_listing(&reader, get)
        .await
        .expect("namespace list");
    assert!(none.namespaces.is_empty() && none.complete);

    monitor.shutdown().await.expect("monitor shutdown");
    drop(tokens);
}

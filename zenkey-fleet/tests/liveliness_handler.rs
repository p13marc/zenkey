//! `spec/scenarios/presence.md` §4 (core §8.1; the watch session's round
//! trip since 0.38, F-186), and spec §8.1's handler rule in the fleet's own
//! liveliness GETs (#612, FJ3).
//!
//! zenoh's default 256-slot handler hangs a liveliness GET on a session that
//! also holds a liveliness subscriber, at every size measured from 996 tokens
//! (zenoh#2678, spike S2). v1's `roster`, `node_info` and `discover_bases`
//! each ran a liveliness GET on zenoh's default handler until this suite,
//! and against that code this test hung: the deadlock blocks the runtime's
//! own threads, so not even a `tokio::time::timeout` around the read fires.
//!
//! Since #830 (core §8.1, 0.37) a session that holds a liveliness
//! subscriber makes no presence read at all — it is a watch session, and
//! the reads go through another — and the one liveliness GET it makes is
//! the round trip that settles its subscribers (`zenkey::presence::settle`,
//! inside [`zenkey_fleet::Monitor::start`]). That GET is §4's: beside a
//! callback subscriber to every token of the population, under the two
//! pairings of handlers the reference makes, it completes, at the routers'
//! final reply and before its timeout, and its answer is discarded. A GET
//! beside a liveliness subscriber can complete without live tokens
//! (Appendix B), so no token of it is counted. §4's pairings with a
//! bounded subscriber nobody drains are not run: they are zenoh's, measured
//! by spike S2 and the second implementation, they wedge zenoh's own
//! threads in process, and the reference declares every liveliness
//! subscriber with a callback.
//!
//! So the body runs on a thread and a runtime of its own, and the test
//! thread waits for it with a deadline: a regression fails here, by name,
//! instead of hanging the suite. A binary of its own, because the tokens
//! load the router.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

mod util;
use util::zk2::{client, router};
use zenkey_fleet::{Monitor, MonitorSpec};

/// §4's population: past zenoh#2678's threshold (996 tokens in spike S2),
/// with margin. §4 asks at least this many (0.38): the hang needs no more,
/// and at 10,000, in a debug build, the setup alone takes some 24 s.
const N: usize = 1_200;

/// Each GET's timeout: a GET that ends before it ended at the routers'
/// final reply.
const GET: Duration = Duration::from_secs(5);

/// §4: the watch session's round trip beside a liveliness subscriber to
/// every token completes, under each pairing of handlers the reference
/// makes, and the reader's own reads on its other session complete with
/// every token.
#[test]
fn s4_reading_presence_at_scale() {
    let (done, wait) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(round_trips_beside_a_subscriber());
        let _ = done.send(());
    });
    // Propagation of the tokens has its own 120 s budget inside the body;
    // past this, the body is hung, not slow.
    match wait.recv_timeout(Duration::from_secs(300)) {
        Ok(()) => {}
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!(
            "a liveliness GET hung on a session holding a liveliness subscriber \
             (spec §8.1, zenoh#2678)"
        ),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            panic!("the body panicked (its message is above)")
        }
    }
}

/// Signals when a GET's callback is dropped, which zenoh does when the GET
/// is finalized: at the routers' final reply, or at its timeout.
struct Finalized(Option<tokio::sync::oneshot::Sender<()>>);

impl Drop for Finalized {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

async fn round_trips_beside_a_subscriber() {
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
    // Setup: the router holds every token, read on the reader's session,
    // which holds no liveliness subscriber (core §8.1).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        let read = zenkey_fleet::liveliness_read(&reader, "demo/held/*/alive", GET)
            .await
            .expect("a liveliness read");
        let seen = read.keys.len();
        if seen == N && read.complete {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{seen} of {N} tokens after 120 s"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // The watch session: a liveliness subscriber with history to every
    // token, a callback, and its round trip on the runtime's handler, an
    // unbounded channel drained as replies arrive (`Monitor::start`).
    let watch = client(&ep).await;
    let delivered = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    let monitor = Monitor::start(
        &watch,
        MonitorSpec {
            liveliness: vec!["demo/held/*/alive".to_owned()],
            ..MonitorSpec::default()
        },
    )
    .await
    .expect("monitor");
    let drained = started.elapsed();
    assert_eq!(
        monitor.settled(),
        Some(true),
        "a drained GET beside a callback subscriber ended at the routers' final reply"
    );
    assert_eq!(
        zenkey::presence::liveliness_gets(&watch),
        zenkey::presence::LivelinessGets {
            reads: 0,
            settles: 1
        },
        "the watch session's one GET"
    );

    // Callbacks for both: the same round trip, on zenoh's own API with a
    // callback GET. It ends before its timeout and with no error reply; its
    // answer is discarded, and nothing of it is counted but how it ended.
    let errors = Arc::new(AtomicU64::new(0));
    let (tx, finalized) = tokio::sync::oneshot::channel();
    let guard = Finalized(Some(tx));
    let e = Arc::clone(&errors);
    let d = Arc::clone(&delivered);
    let asked = Instant::now();
    watch
        .liveliness()
        .get("demo/held/*/alive")
        .timeout(GET)
        .callback(move |reply| {
            let _ = &guard;
            if reply.result().is_err() {
                e.fetch_add(1, Ordering::Relaxed);
            } else {
                d.fetch_add(1, Ordering::Relaxed);
            }
        })
        .await
        .expect("a callback GET");
    tokio::time::timeout(2 * GET, finalized)
        .await
        .expect("finalized within twice its timeout")
        .expect("finalized");
    let callbacks = asked.elapsed();
    assert!(
        callbacks < GET,
        "the callback GET ended at the routers' final reply, before its timeout: {callbacks:?}"
    );
    assert_eq!(errors.load(Ordering::Relaxed), 0, "no error reply");
    eprintln!(
        "§4 at {N} tokens: the drained round trip in {drained:?} (with the subscriber's \
         declaration), the callback one in {callbacks:?}"
    );

    // A presence read on the watch session is refused, nothing asked (#830).
    assert!(
        zenkey_fleet::liveliness_read(&watch, "demo/held/*/alive", GET)
            .await
            .expect_err("refused")
            .not_asked()
            .is_some()
    );

    // A read across every prefix, the shape `namespace list` asks
    // (`**/zk2/…`), through the same chokepoint, on the reader's session:
    // every token, and complete.
    let read = zenkey_fleet::liveliness_read(&reader, "**/held/*/alive", GET)
        .await
        .expect("a liveliness read");
    assert_eq!(read.keys.len(), N);
    assert!(read.complete, "a GET that returned before its timeout");
    let none = zenkey_fleet::namespace_listing(&reader, GET)
        .await
        .expect("namespace list");
    assert!(none.namespaces.is_empty() && none.complete);

    monitor.shutdown().await.expect("monitor shutdown");
    drop(tokens);
}

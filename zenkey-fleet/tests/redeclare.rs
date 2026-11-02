//! `spec/scenarios/constrained.md` §9 step 2 (core §8.5, observer §7.5;
//! #988 L4): a tool's watch across a face whose medium repairs no lost
//! frame is declared again after two horizons of silence, and heals.
//!
//! The far client dials the router through a relay that drops the next
//! batch it sends toward the router but a keep-alive, and forwards
//! everything else, its transport left up: a frame lost below zenoh, as a
//! radio with no ARQ loses one. The key expression crossed before, with a
//! first subscription, so the batch lost is the watch's
//! `DeclareSubscriber`; a lost `DeclareKeyExpr` is never declared again on
//! a transport (core Appendix B), as `zenkey/tests/constrained.rs` §9
//! shows.
//!
//! The same relay also stalls (#1047): it stops reading what the far
//! client sends, as a slow link backs up, so a declaration the far client
//! makes waits for room in its transmission queue, and one after it waits
//! for the first (zenoh holds its routing's control lock while a new key's
//! `DeclareKeyExpr` waits). A re-declare in flight there holds no lock that
//! `unwatch` waits on, and no runtime worker.

mod util;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use util::zk2::{client, router};
use zenkey_fleet::{FleetEvent, Monitor, MonitorSpec, StreamItem};

/// A TCP relay to `upstream` that, while armed, drops the next batch the
/// dialling side sends but a keep-alive (each batch after the stream link's
/// 2-byte little-endian length), once; and, while stalled, reads nothing the
/// dialling side sends.
struct LossyRelay {
    endpoint: String,
    armed: Arc<Mutex<bool>>,
    lost: Arc<AtomicU64>,
    stalled: Arc<AtomicBool>,
    _accept: tokio::task::JoinHandle<()>,
}

impl LossyRelay {
    async fn to(upstream: &str) -> LossyRelay {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let target = upstream.trim_start_matches("tcp/").to_owned();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("tcp/{}", listener.local_addr().unwrap());
        let armed: Arc<Mutex<bool>> = Arc::default();
        let lost = Arc::new(AtomicU64::new(0));
        let stalled: Arc<AtomicBool> = Arc::default();
        let (a, l, st) = (Arc::clone(&armed), Arc::clone(&lost), Arc::clone(&stalled));
        let accept = tokio::spawn(async move {
            while let Ok((down, _)) = listener.accept().await {
                let Ok(up) = tokio::net::TcpStream::connect(&target).await else {
                    continue;
                };
                let (mut down_rd, mut down_wr) = down.into_split();
                let (mut up_rd, mut up_wr) = up.into_split();
                let (a, l, st) = (Arc::clone(&a), Arc::clone(&l), Arc::clone(&st));
                tokio::spawn(async move {
                    let _ = tokio::io::copy(&mut up_rd, &mut down_wr).await;
                });
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65_536];
                    let mut pending: Vec<u8> = Vec::new();
                    loop {
                        while st.load(Ordering::SeqCst) {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                        let n = match down_rd.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => n,
                        };
                        pending.extend_from_slice(&buf[..n]);
                        let mut out = Vec::new();
                        while pending.len() >= 2 {
                            let len = usize::from(u16::from_le_bytes([pending[0], pending[1]]));
                            if pending.len() < 2 + len {
                                break;
                            }
                            let batch: Vec<u8> = pending.drain(..2 + len).collect();
                            let mut armed = a.lock().unwrap();
                            // zenoh 1.10.1's `KeepAlive`: one byte, 0x04.
                            let keep_alive = batch.len() == 3 && batch[2] == 0x04;
                            if *armed && !keep_alive {
                                *armed = false;
                                l.fetch_add(1, Ordering::SeqCst);
                            } else {
                                out.extend_from_slice(&batch);
                            }
                        }
                        if up_wr.write_all(&out).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
        LossyRelay {
            endpoint,
            armed,
            lost,
            stalled,
            _accept: accept,
        }
    }

    fn lose_next(&self) {
        *self.armed.lock().unwrap() = true;
    }

    fn stall(&self, stalled: bool) {
        self.stalled.store(stalled, Ordering::SeqCst);
    }
}

/// §9 step 2: the relay loses the batch carrying the far monitor's
/// declaration of a stream put every 100 ms, whose horizon is 1 s. Nothing
/// arrives for two horizons; the watch is then declared again, once, and
/// the stream arrives from then on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s9_a_watch_across_a_face_is_declared_again_after_two_silent_horizons() {
    let (_router, ep) = router(None).await;
    let owner = client(&ep).await;
    let key = "zk2/v/beacon/beacon.v1/stream/level";
    let publisher = owner.declare_publisher(key).await.unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let s = Arc::clone(&stop);
    let puts = tokio::spawn(async move {
        while !s.load(Ordering::SeqCst) {
            let _ = publisher.put("1").await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    let relay = LossyRelay::to(&ep).await;
    let far = zenkey_fleet::bus::session::open(std::slice::from_ref(&relay.endpoint), &[], false)
        .await
        .expect("a far client");
    // The key expression crosses with a first subscription, which hears the
    // stream and goes.
    let first = far.declare_subscriber(key).await.unwrap();
    tokio::time::timeout(util::SETTLE, first.recv_async())
        .await
        .expect("the first subscription hears the stream")
        .unwrap();
    first.undeclare().await.unwrap();
    let monitor = Monitor::start(&far, MonitorSpec::default()).await.unwrap();
    let mut events = monitor.events();
    tokio::time::sleep(Duration::from_millis(200)).await;
    relay.lose_next();
    let t = Instant::now();
    monitor
        .watch_across(key, Duration::from_secs(1))
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    let first = loop {
        let item = tokio::time::timeout_at(deadline, events.recv())
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "the stream never crossed: lost {}, declared again {}",
                    relay.lost.load(Ordering::SeqCst),
                    monitor.redeclared()
                )
            })
            .expect("the monitor runs");
        if let StreamItem::Event(FleetEvent::Sample(_)) = item {
            break t.elapsed();
        }
    };
    assert_eq!(
        relay.lost.load(Ordering::SeqCst),
        1,
        "the declaration was lost"
    );
    assert!(
        first >= Duration::from_secs(2),
        "nothing for two horizons: the first sample at {first:?}"
    );
    assert_eq!(monitor.redeclared(), 1);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        monitor.redeclared(),
        1,
        "a watch that hears is never re-declared"
    );
    assert_eq!(monitor.watched().await.len(), 1);
    stop.store(true, Ordering::SeqCst);
    puts.await.unwrap();
    monitor.shutdown().await.unwrap();
}

/// A far client of the relay whose declarations wait behind its data
/// (#1047): no QoS, so one transmission queue holds both, and a
/// `wait_before_close` and a lease that outlast the stall, so the transport
/// stays up through it.
async fn far_client(endpoint: &str) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    for (key, value) in [
        ("mode", "\"client\"".to_owned()),
        ("connect/endpoints", format!("[\"{endpoint}\"]")),
        ("scouting/multicast/enabled", "false".to_owned()),
        ("scouting/gossip/enabled", "false".to_owned()),
        ("transport/unicast/qos/enabled", "false".to_owned()),
        ("transport/link/tx/lease", "60000".to_owned()),
        (
            "transport/link/tx/queue/congestion_control/block/wait_before_close",
            "60000000".to_owned(),
        ),
    ] {
        c.insert_json5(key, &value).expect("config");
    }
    zenoh::open(c).await.expect("a far client")
}

/// #1047: a watch across a face is being declared again over a link backed
/// up behind blocking puts and another declaration, and `unwatch` returns
/// while that declaration waits; once the link drains, the subscriber
/// declared for the watch removed is not stored, so nothing counts as
/// declared again and no watch remains. (With the slots lock held across
/// the declaration, `unwatch` waits for the link: this test times out.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unwatch_returns_while_a_watch_is_declared_again_across_a_slow_link() {
    let (router, ep) = router(None).await;
    // Something hears the puts at the router, so the far client sends them.
    let bulk = "zk2/v/flood/flood.v1/stream/bulk";
    let _hears = router
        .declare_subscriber(bulk)
        .callback(|_| {})
        .await
        .unwrap();
    let relay = LossyRelay::to(&ep).await;
    let far = far_client(&relay.endpoint).await;
    let key = "zk2/v/beacon/beacon.v1/stream/level";
    let monitor = Monitor::start(&far, MonitorSpec::default()).await.unwrap();
    // Silent from the start: declared again two horizons on.
    let horizon = Duration::from_secs(2);
    let id = monitor.watch_across(key, horizon).await.unwrap();
    let t = Instant::now();

    // The link backs up: the relay reads nothing, and blocking puts fill
    // the far client's socket and its queue, on a thread of their own.
    relay.stall(true);
    let stop = Arc::new(AtomicBool::new(false));
    let sent = Arc::new(AtomicU64::new(0));
    let (s, n) = (Arc::clone(&stop), Arc::clone(&sent));
    let publisher = far
        .declare_publisher(bulk)
        .congestion_control(zenoh::qos::CongestionControl::Block)
        .await
        .unwrap();
    let flood = std::thread::spawn(move || {
        use zenoh::Wait as _;
        let payload = vec![0u8; 32 * 1024];
        while !s.load(Ordering::SeqCst) {
            if publisher.put(payload.clone()).wait().is_err() {
                break;
            }
            n.fetch_add(1, Ordering::SeqCst);
        }
    });
    // Backed up, well before two horizons: no put has gone out for 300 ms.
    let mut last = u64::MAX;
    loop {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let now = sent.load(Ordering::SeqCst);
        if now == last {
            break;
        }
        assert!(
            t.elapsed() < horizon,
            "the link backs up within a horizon: {now} puts"
        );
        last = now;
    }

    // A declaration ahead of the re-declare waits for room in the queue,
    // on a thread of its own: the session's declarations wait behind it.
    let ahead = {
        let far = far.clone();
        std::thread::spawn(move || {
            use zenoh::Wait as _;
            far.declare_subscriber("zk2/v/other/other.v1/stream/level")
                .wait()
                .and_then(|s| s.undeclare().wait())
                .is_ok()
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!ahead.is_finished(), "a declaration waits behind the puts");

    // Two horizons on, the check undeclares the silent watch's subscriber
    // to declare it again (a tick is a tenth of the silence), and waits
    // behind the declaration ahead.
    tokio::time::sleep((2 * horizon + Duration::from_secs(1)).saturating_sub(t.elapsed())).await;
    assert_eq!(monitor.redeclared(), 0, "the declaration waits");
    let unwatched = Instant::now();
    tokio::time::timeout(Duration::from_secs(2), monitor.unwatch(id))
        .await
        .expect("unwatch returns while the declaration waits")
        .unwrap();
    assert!(unwatched.elapsed() < Duration::from_secs(2));
    assert!(monitor.watched().await.is_empty());

    // The link drains: the declaration made for the watch removed goes.
    relay.stall(false);
    stop.store(true, Ordering::SeqCst);
    flood.join().unwrap();
    assert!(ahead.join().unwrap(), "the declaration ahead is made");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        (monitor.redeclared(), monitor.redeclare_failed()),
        (0, 0),
        "the watch removed is not stored again"
    );
    monitor.shutdown().await.unwrap();
}

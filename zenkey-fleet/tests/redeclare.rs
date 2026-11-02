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

mod util;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use util::zk2::{client, router};
use zenkey_fleet::{FleetEvent, Monitor, MonitorSpec, StreamItem};

/// A TCP relay to `upstream` that, while armed, drops the next batch the
/// dialling side sends but a keep-alive (each batch after the stream link's
/// 2-byte little-endian length), once.
struct LossyRelay {
    endpoint: String,
    armed: Arc<Mutex<bool>>,
    lost: Arc<AtomicU64>,
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
        let (a, l) = (Arc::clone(&armed), Arc::clone(&lost));
        let accept = tokio::spawn(async move {
            while let Ok((down, _)) = listener.accept().await {
                let Ok(up) = tokio::net::TcpStream::connect(&target).await else {
                    continue;
                };
                let (mut down_rd, mut down_wr) = down.into_split();
                let (mut up_rd, mut up_wr) = up.into_split();
                let (a, l) = (Arc::clone(&a), Arc::clone(&l));
                tokio::spawn(async move {
                    let _ = tokio::io::copy(&mut up_rd, &mut down_wr).await;
                });
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65_536];
                    let mut pending: Vec<u8> = Vec::new();
                    loop {
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
            _accept: accept,
        }
    }

    fn lose_next(&self) {
        *self.armed.lock().unwrap() = true;
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

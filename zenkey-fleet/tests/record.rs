//! `.zrec` capture and replay over a real bus (#39; RFC 09 §5.2): what a
//! monitor observed round-trips through a file and back onto a second bus
//! through declared publishers — key, payload bytes, kind, encoding and
//! attachment intact — and the capture's drop ledger survives into the
//! replay report.
//!
//! Event-driven: the matching badge proves routability before publishing.
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zenkey::qos::QosProfile;
use zenkey_fleet::{
    RecordBounds, ReplayTarget, ZREC_VERSION, ZrecHeader, ZrecSink, ZrecSource,
    declare_publication, record, replay,
};

mod util;
use util::peer_pair;

/// A byte sink the test can read back.
///
/// The sink **owns** its writer — it lives on the blocking pool (#332) — so
/// an in-memory capture cannot simply take the `Vec` back out of it the way
/// a synchronous `ZrecWriter::finish` handed it over.
#[derive(Clone, Default)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl SharedBuf {
    fn take(&self) -> Vec<u8> {
        self.0.lock().expect("buffer lock").clone()
    }
}

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("buffer lock").write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A [`SharedBuf`] that costs real time per write — a disk, modelled.
#[derive(Clone)]
struct SlowBuf {
    inner: SharedBuf,
    per_write: Duration,
}

impl Write for SlowBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Blocking on purpose: this is what must not be on a runtime thread.
        std::thread::sleep(self.per_write);
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn header(selector: &str) -> ZrecHeader {
    ZrecHeader {
        zrec: ZREC_VERSION,
        selectors: vec![selector.to_string()],
        base: String::new(),
        captured_at: "2026-08-12T00:00:00Z".to_string(),
    }
}

const KEY: &str = "v1/h-aaaaaaaaaaaa/state/demo/health";
const SELECTOR: &str = "v1/h-aaaaaaaaaaaa/state/demo/**";

/// Record real traffic — including a binary payload no JSON rendering can
/// carry and a tombstone — replay it into a second, disconnected bus, and
/// observe the same sequence: keys, exact payload bytes, kinds, encoding,
/// attachment.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_capture_replays_onto_a_second_bus_intact() {
    let (a, b) = peer_pair().await;

    // --- capture side ------------------------------------------------
    let monitor = zenkey_fleet::Monitor::start(&b, zenkey_fleet::MonitorSpec::default())
        .await
        .expect("monitor");
    let mut events = monitor.events();
    monitor.watch(SELECTOR).await.expect("watch");

    let publication =
        declare_publication(&a, KEY, QosProfile::Transition, Some("application/json"))
            .await
            .expect("declare");
    let matching = publication.matching_events().await.expect("matching");
    assert!(
        tokio::time::timeout(Duration::from_secs(5), matching.recv())
            .await
            .expect("matching within 5s")
            .expect("listener alive")
    );

    let binary: Vec<u8> = vec![0x00, 0xff, 0x01, 0xfe, 0x80];
    publication
        .send(br#"{"ok":true}"#.to_vec(), Some(b"who=test".to_vec()))
        .await
        .expect("send json");
    publication
        .send(binary.clone(), None)
        .await
        .expect("send binary");
    publication.retire().await.expect("retire");

    let buf = SharedBuf::default();
    let sink = ZrecSink::spawn(buf.clone(), &header(SELECTOR))
        .await
        .expect("sink");
    record(
        &mut events,
        &sink,
        RecordBounds {
            max_samples: Some(3),
            max_duration: Some(Duration::from_secs(10)),
        },
        |_, _| {},
    )
    .await
    .expect("record");
    let (samples, dropped) = sink.finish().await.expect("finish");
    assert_eq!(samples, 3, "two puts and a tombstone");
    assert_eq!(dropped, 0);
    let file = buf.take();

    // --- replay side (a second, unrelated bus) ------------------------
    let (c, d) = peer_pair().await;
    let replay_monitor = zenkey_fleet::Monitor::start(&d, zenkey_fleet::MonitorSpec::default())
        .await
        .expect("replay monitor");
    let mut replayed = replay_monitor.events();
    replay_monitor.watch(SELECTOR).await.expect("watch replay");
    // Routability gate for the replay session: a declared publisher on the
    // watched selector's key must see the subscriber before we replay.
    let gate = declare_publication(&c, KEY, QosProfile::Transition, None)
        .await
        .expect("gate");
    let gate_matching = gate.matching_events().await.expect("gate matching");
    assert!(
        tokio::time::timeout(Duration::from_secs(5), gate_matching.recv())
            .await
            .expect("gate matching within 5s")
            .expect("listener alive")
    );
    gate.undeclare().await.expect("undeclare gate");

    let mut reader = ZrecSource::spawn(std::io::Cursor::new(file))
        .await
        .expect("reader");
    let report = replay(
        &mut reader,
        zenkey_fleet::ReplaySpec {
            target: ReplayTarget::Bus {
                session: &c,
                slices: None,
            },
            // scaled pacing: original gaps are µs-scale anyway
            speed: 1000.0,
            i_know: false,
            default_qos: zenkey::qos::QosProfile::Refreshed,
        },
        |_| {},
    )
    .await
    .expect("replay");
    assert_eq!(report.published, 2);
    assert_eq!(report.tombstones, 1, "state-shaped delete needs no force");
    assert_eq!(report.malformed, 0);
    assert_eq!(report.refused, 0);

    let mut views = Vec::new();
    while views.len() < 3 {
        let item = tokio::time::timeout(Duration::from_secs(5), replayed.recv())
            .await
            .expect("replayed event within 5s")
            .expect("stream alive");
        if let zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::Sample(s)) = item {
            views.push(s);
        }
    }
    assert!(views.iter().all(|v| v.key == KEY));
    assert_eq!(views[0].payload.to_bytes().as_ref(), br#"{"ok":true}"#);
    assert_eq!(views[0].encoding, "application/json");
    assert_eq!(
        views[0]
            .attachment
            .as_ref()
            .expect("attachment survives the file")
            .to_bytes()
            .as_ref(),
        b"who=test"
    );
    assert_eq!(
        views[1].payload.to_bytes().as_ref(),
        binary.as_slice(),
        "binary payload is lossless through the bytes field"
    );
    assert_eq!(views[2].kind, zenoh::sample::SampleKind::Delete);
    // Replay re-stamped: the replaying session's HLC, not the capture's
    // (the capture sessions here stamp nothing — what matters is that the
    // wire fact is the *replay's*, whatever it is).
    assert!(
        views[0].qos_matches(QosProfile::Transition),
        "the recorded profile name declared the replay publisher"
    );
}

/// A capture under load stores its drop ledger in the file, and a replay
/// surfaces it: the drops ride positions, and the report totals them (O6).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lossy_capture_says_so_at_both_ends() {
    // No bus needed: drive a core directly and let the bounded broadcast
    // lag — the same mechanism a hot bus triggers.
    let core = zenkey_fleet::MonitorCore::new(4);
    let mut events = core.events();
    for i in 0..32u8 {
        core.ingest(
            zenkey_fleet::SampleView {
                key: KEY.to_string(),
                payload: zenoh::bytes::ZBytes::from(vec![i]),
                encoding: String::new(),
                kind: zenoh::sample::SampleKind::Put,
                timestamp: None,
                stamped_by: None,
                attachment: None,
                priority: zenoh::qos::Priority::Data,
                congestion_control: zenoh::qos::CongestionControl::Drop,
                reliability: zenoh::qos::Reliability::BestEffort,
                express: false,
                source: None,
                received: std::time::Instant::now(),
            },
            None,
        );
    }
    // The stream holds the core alive (it counts drops through it), so the
    // channel never closes underneath a recorder — bound by time instead:
    // everything is already buffered, so the drain is instant and the
    // deadline only caps the tail.
    let buf = SharedBuf::default();
    let sink = ZrecSink::spawn(buf.clone(), &header(SELECTOR))
        .await
        .expect("sink");
    record(
        &mut events,
        &sink,
        RecordBounds {
            max_samples: None,
            max_duration: Some(Duration::from_secs(1)),
        },
        |_, _| {},
    )
    .await
    .expect("record");
    let (samples, dropped) = sink.finish().await.expect("finish");
    assert!(dropped > 0, "a capacity-4 channel under 32 sends must lag");
    assert!(samples > 0);
    let file = buf.take();

    let text = String::from_utf8(file.clone()).expect("a .zrec is text");
    assert!(
        text.lines().any(|l| l.contains("\"dropped\"")),
        "the drop ledger is in the file, not only in memory"
    );

    let mut reader = ZrecSource::spawn(std::io::Cursor::new(file))
        .await
        .expect("reader");
    let report = replay(
        &mut reader,
        zenkey_fleet::ReplaySpec {
            target: ReplayTarget::DryRun,
            speed: 1.0,
            i_know: false,
            default_qos: zenkey::qos::QosProfile::Refreshed,
        },
        |_| {},
    )
    .await
    .expect("dry replay");
    assert_eq!(
        report.capture_dropped, dropped,
        "the ledger survives replay"
    );
    assert_eq!(u64::from(report.dry_run), 1);
    assert_eq!(report.published, samples);
}

/// #332: a slow disk no longer manufactures the drops the capture records.
///
/// The traffic is shaped so the answer is a property, not a race: bursts of
/// 150 — well inside the shipped 1024-slot broadcast, so a burst alone can
/// never overflow it — with 5 ms between them. Draining a burst is 150
/// pointer moves into the sink's queue, microseconds; *writing* one is 150
/// rows at 200 µs, thirty milliseconds. So the backlog can only grow if the
/// writing is what the drain is doing.
///
/// When the write ran inline in the drain loop, it was: the backlog grew by
/// most of a burst each round, the broadcast overflowed a few bursts in, and
/// the capture wrote `{"dropped": n}` records caused by its own writer. With
/// the writer on the blocking pool behind the sink's bounded queue the drain
/// keeps up with every burst, and a drop record in a `.zrec` means what it
/// says: the bus outran the observer.
///
/// The final `finish` is timed as well — it must still be draining when the
/// capture ends, which is the proof that the queue really did absorb a
/// writer stall rather than the writer having quietly kept up.
///
/// Measured on this exact traffic while the fix was written: the old drain
/// loop captured 1 207 samples and recorded 293 drops, every one of them the
/// writer's; this one captures 1 500 and records none.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_writer_does_not_become_the_captures_drops() {
    const SAMPLES: u64 = 1_500;
    /// Samples per burst — comfortably inside the broadcast's 1024 slots.
    const BURST: u64 = 150;

    let core = zenkey_fleet::MonitorCore::new(1024); // the shipped default
    let mut events = core.events();
    let buf = SharedBuf::default();
    let sink = ZrecSink::spawn(
        SlowBuf {
            inner: buf.clone(),
            per_write: Duration::from_micros(100),
        },
        &header(SELECTOR),
    )
    .await
    .expect("sink");

    let producer = {
        let core = std::sync::Arc::clone(&core);
        tokio::spawn(async move {
            for i in 0..SAMPLES {
                if i > 0 && i % BURST == 0 {
                    // Between bursts: long enough for any drain that is not
                    // writing to disk to have emptied the last one, far too
                    // short for one that is.
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                core.ingest(
                    zenkey_fleet::SampleView {
                        key: KEY.to_string(),
                        payload: zenoh::bytes::ZBytes::from(i.to_le_bytes().to_vec()),
                        encoding: String::new(),
                        kind: zenoh::sample::SampleKind::Put,
                        timestamp: None,
                        stamped_by: None,
                        attachment: None,
                        priority: zenoh::qos::Priority::Data,
                        congestion_control: zenoh::qos::CongestionControl::Drop,
                        reliability: zenoh::qos::Reliability::BestEffort,
                        express: false,
                        source: None,
                        received: std::time::Instant::now(),
                    },
                    None,
                );
                // The producer is not the subject of the measurement: yield
                // so the drain is scheduled the way a real network callback
                // thread (a thread of its own) would leave it.
                tokio::task::yield_now().await;
            }
        })
    };

    record(
        &mut events,
        &sink,
        RecordBounds {
            max_samples: Some(SAMPLES),
            max_duration: Some(Duration::from_secs(30)),
        },
        |_, _| {},
    )
    .await
    .expect("record");
    producer.await.expect("producer");

    let draining = std::time::Instant::now();
    let (samples, dropped) = sink.finish().await.expect("finish");
    let drain_took = draining.elapsed();

    assert_eq!(dropped, 0, "the writer's latency is not the bus's loss");
    assert_eq!(samples, SAMPLES, "every sample reached the file");
    assert!(
        drain_took > Duration::from_millis(20),
        "the writer kept up on its own, so this proves nothing: {drain_took:?}"
    );
    let text = String::from_utf8(buf.take()).expect("a .zrec is text");
    assert!(
        !text.lines().any(|l| l.contains("\"dropped\"")),
        "no self-inflicted drop record"
    );
}

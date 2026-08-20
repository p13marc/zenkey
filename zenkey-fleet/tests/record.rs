//! `.zrec` capture and replay over a real bus (#39; RFC 09 §5.2): what a
//! monitor observed round-trips through a file and back onto a second bus
//! through declared publishers — key, payload bytes, kind, encoding and
//! attachment intact — and the capture's drop ledger survives into the
//! replay report.
//!
//! Event-driven: the matching badge proves routability before publishing.
//! Ports 7524-7525 (disjoint from every other test binary).

use std::time::Duration;

use zenkey::qos::QosProfile;
use zenkey_fleet::{
    RecordBounds, ReplayTarget, ZREC_VERSION, ZrecHeader, ZrecReader, ZrecWriter,
    declare_publication, record, replay,
};

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
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
    let (a, b) = peer_pair(7524).await;

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

    let mut writer = ZrecWriter::new(Vec::new(), &header(SELECTOR)).expect("writer");
    record(
        &mut events,
        &mut writer,
        RecordBounds {
            max_samples: Some(3),
            max_duration: Some(Duration::from_secs(10)),
        },
        |_, _| {},
    )
    .await
    .expect("record");
    let (samples, dropped) = writer.counts();
    assert_eq!(samples, 3, "two puts and a tombstone");
    assert_eq!(dropped, 0);
    let file = writer.finish().expect("finish");

    // --- replay side (a second, unrelated bus) ------------------------
    let (c, d) = peer_pair(7525).await;
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

    let mut reader = ZrecReader::new(file.as_slice()).expect("reader");
    let report = replay(
        &mut reader,
        ReplayTarget::Bus {
            session: &c,
            slices: None,
        },
        1000.0, // scaled pacing: original gaps are µs-scale anyway
        false,
        "refreshed",
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
    let mut writer = ZrecWriter::new(Vec::new(), &header(SELECTOR)).expect("writer");
    record(
        &mut events,
        &mut writer,
        RecordBounds {
            max_samples: None,
            max_duration: Some(Duration::from_secs(1)),
        },
        |_, _| {},
    )
    .await
    .expect("record");
    let (samples, dropped) = writer.counts();
    assert!(dropped > 0, "a capacity-4 channel under 32 sends must lag");
    assert!(samples > 0);
    let file = writer.finish().expect("finish");

    let text = String::from_utf8(file.clone()).expect("a .zrec is text");
    assert!(
        text.lines().any(|l| l.contains("\"dropped\"")),
        "the drop ledger is in the file, not only in memory"
    );

    let mut reader = ZrecReader::new(file.as_slice()).expect("reader");
    let report = replay(
        &mut reader,
        ReplayTarget::DryRun,
        1.0,
        false,
        "refreshed",
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

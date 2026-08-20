//! The stamper is not always the publisher (issue #213, RFC 09 §5.1 **O7**).
//!
//! The unit tests in `stats.rs` prove the three populations stay apart once
//! they are classified. This pins the *classification* against a real bus, and
//! in particular pins the limit that measuring found:
//!
//! **zenoh 1.9 delivers no `SourceInfo` to a subscriber** — not from a plain
//! publisher and not from an AdvancedPublisher. So the id-to-id comparison the
//! classifier wants is usually unavailable, and the honest answer is
//! `Unattributable`: we know *a* clock stamped the sample and which one, and
//! we cannot say whether it was the publisher's.
//!
//! That is a weaker claim than the one the code used to make, and it is the
//! true one. The test below proves the classifier is *conservative* rather
//! than wrong: it checks independently that the stamper id really does equal
//! the publishing session's zid, while our verdict stays "cannot attribute".
//! The day SourceInfo rides — an explicit `source_info()` on the publisher, or
//! a future zenoh that propagates it — this test is what will notice.

use std::time::Duration;

use zenkey_fleet::{FleetEvent, Monitor, MonitorSpec, StampProvenance, StreamItem};

/// A session that stamps what it publishes, as a fleet with
/// `timestamping.enabled` does.
async fn timestamping_listener(port: u16) -> zenoh::Session {
    let mut cfg = zenoh::Config::default();
    cfg.insert_json5("scouting/multicast/enabled", "false").ok();
    cfg.insert_json5("timestamping/enabled", "true").ok();
    cfg.insert_json5("listen/endpoints", &format!("[\"tcp/127.0.0.1:{port}\"]"))
        .ok();
    zenoh::open(cfg).await.expect("publisher session")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stamp_we_cannot_attribute_is_unknown_and_still_names_its_stamper() {
    let key = "v1/h-3fa9c2d41b7e/state/stamp/probe";
    let publisher = timestamping_listener(7483).await;
    let publisher_zid = publisher.zid();
    let observer = zenkey_fleet::open(&["tcp/127.0.0.1:7483".to_string()], &[], false)
        .await
        .expect("observer session");

    let monitor = Monitor::start(&observer, MonitorSpec::default())
        .await
        .expect("monitor");
    let mut events = monitor.events();
    monitor.watch(key).await.expect("watch");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let pubr = publisher
        .declare_publisher(key)
        .await
        .expect("declare publisher");
    for _ in 0..5 {
        pubr.put("1").await.expect("put");
        tokio::time::sleep(Duration::from_millis(40)).await;
    }

    let mut seen = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), events.recv()).await {
            Ok(Some(StreamItem::Event(FleetEvent::Sample(s))))
                if s.key == key && s.timestamp.is_some() =>
            {
                seen = Some((s.stamped_by, s.source));
                break;
            }
            Ok(Some(_)) => continue,
            _ => break,
        }
    }
    monitor.stop();

    let (provenance, source) = seen.expect("a stamped sample within 5s");
    let provenance = provenance.expect("a stamped sample has a provenance");

    assert!(
        source.is_none(),
        "zenoh 1.9 delivers no SourceInfo to a subscriber. If this now fails, \
         SourceInfo has started riding and the classifier can attribute for \
         real — update this test and O7's practical note rather than the code."
    );

    match provenance {
        StampProvenance::Unattributable { stamper } => {
            // The independent check: the stamper *was* the publisher. Our
            // verdict says "cannot tell", which is conservative — never a
            // guess in either direction (RFC 09 §5.1 O4 applied to a clock).
            assert_eq!(
                stamper,
                zenoh::time::TimestampId::from(publisher_zid),
                "the publishing session did stamp this; we simply had no \
                 SourceInfo on the wire with which to establish it"
            );
        }
        other => panic!(
            "with no SourceInfo the comparison cannot be made, so the only \
             honest verdict is Unattributable: got {other:?}"
        ),
    }
}

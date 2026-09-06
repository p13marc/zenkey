//! The timeline against a real bus (#216): what the live path classifies.
//!
//! `tests/stamper.rs` pins the limit — zenoh 1.9 and 1.10 deliver no
//! `SourceInfo` to a subscriber — at the `StampProvenance` seam. This pins
//! the same limit one layer up, where a timeline reads it: a stamped row's
//! provenance is `Unattributable` with the stamping session's id as the
//! stamper, and the sequence-number lane is *unavailable*, structurally,
//! never an empty vector. The day SourceInfo rides, both tests move
//! together.

use std::time::{Duration, Instant};

use zenkey_fleet::report::{HlcClaim, LaneId, Provenance, SnLaneReport, TimelineSource};
use zenkey_fleet::{
    Break, FleetEvent, Monitor, MonitorSpec, Order, PlacedBreak, StreamItem, TimelineRow, Window,
    timeline,
};

mod util;
use util::timestamping_pair;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_window_classifies_its_stampers_and_reports_the_sn_lane_unavailable() {
    let key = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/probe";
    let (publisher, observer) = timestamping_pair().await;
    let stamper = zenoh::time::TimestampId::from(publisher.zid()).to_string();

    let monitor = Monitor::start(&observer, MonitorSpec::default())
        .await
        .expect("monitor");
    let mut events = monitor.events();
    monitor.watch(key).await.expect("watch");
    let epoch = Instant::now();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let pubr = publisher
        .declare_publisher(key)
        .await
        .expect("declare publisher");
    for i in 0..5u8 {
        pubr.put(vec![i]).await.expect("put");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut rows = Vec::new();
    let mut breaks = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while rows.len() < 5 && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), events.recv()).await {
            Ok(Some(StreamItem::Event(FleetEvent::Sample(s)))) if s.key == key => {
                rows.push(TimelineRow::from_view(&s, epoch, ""));
            }
            Ok(Some(StreamItem::Dropped(n))) => breaks.push(PlacedBreak {
                after: rows.len(),
                lane: None,
                kind: Break::Dropped(n),
            }),
            Ok(Some(_)) => continue,
            _ => break,
        }
    }
    let keys_evicted = monitor.core().keys_evicted();
    monitor.stop();
    assert_eq!(rows.len(), 5, "five stamped samples within 5s");

    let window = Window {
        rows,
        breaks,
        scopes: vec![key.into()],
        window_s: Some(1.0),
        source: TimelineSource::Live,
        keys_evicted,
    };
    let by_hlc = timeline(&window, Order::Hlc);

    // The limit, read at this layer: stamped, stamper named, attribution
    // withheld (O4 applied to a clock) — and no sequence numbers to lane.
    assert_eq!(by_hlc.unstamped_excluded, 0);
    assert_eq!(by_hlc.lanes.len(), 1);
    let lane = &by_hlc.lanes[0];
    assert_eq!(
        lane.lane,
        LaneId::Origin {
            origin: "h-3fa9c2d41b7e".into(),
            producer: Some("sysinfo".into())
        }
    );
    assert_eq!(lane.provenance.unattributable, 5);
    assert_eq!(lane.provenance.self_stamped, 0);
    assert_eq!(lane.stampers.iter().collect::<Vec<_>>(), [&stamper]);
    assert_eq!(
        by_hlc.axis,
        zenkey_fleet::report::AxisLabel::Hlc {
            claim: HlcClaim::HappensBefore {
                stamper: stamper.clone()
            }
        }
    );
    assert!(
        matches!(by_hlc.sn_lane, SnLaneReport::Unavailable { .. }),
        "zenoh 1.9/1.10 deliver no SourceInfo to a subscriber; if this now \
         fails, the lane has become available — move tests/stamper.rs with it"
    );
    for row in &window.rows {
        assert_eq!(
            row.hlc.as_ref().map(|h| h.provenance),
            Some(Provenance::Unattributable)
        );
        assert!(row.sn.is_none());
    }

    // One stamper, five puts in order: the HLC listing and the arrival
    // listing agree here, which is the *absence* of a reorder — and both
    // say which axis they are on.
    let by_arrival = timeline(&window, Order::Arrival);
    let keys_on = |r: &zenkey_fleet::TimelineReport| {
        r.rows
            .iter()
            .filter_map(|e| match e {
                zenkey_fleet::report::TimelineEntry::Sample { t_us, .. } => Some(*t_us),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(keys_on(&by_hlc), keys_on(&by_arrival));
}

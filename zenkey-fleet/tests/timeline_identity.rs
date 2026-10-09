//! The same window, live and from the file, is the same timeline (#216).
//!
//! The acceptance criterion behind "nothing in `model/` takes a session":
//! a `.zrec` replays through the same projection as live traffic, and the
//! two reports are *equal* — not similar, not the same length. Rows are
//! built from `SampleView`s, written through `ZrecWriter::new_at` with the
//! same epoch the live rows measure from, read back with `ZrecReader`, and
//! projected both ways on both axes.
//!
//! zk2's since #612 (FJ8b): both paths resolve each key through one
//! [`Lens`] — here the tcgui contract held offline, as `zenctl timeline
//! --from <file> --contracts` reads a capture — so the lanes are zk2
//! resources, and the lens is the one input the two sources share.

use std::time::{Duration, Instant};

use zenkey_fleet::report::{LaneId, TimelineSource};
use zenkey_fleet::{
    Break, ContractSet, Ingested, Lens, Order, PlacedBreak, SampleView, StampProvenance,
    StreamItem, TimelineRow, Window, ZREC_VERSION, ZrecHeader, ZrecItem, ZrecReader, ZrecWriter,
    timeline,
};

const BASE: &str = "acme";

/// The tcgui pilot's `tc.netif.v1`, held offline.
fn contracts() -> ContractSet {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../examples/zk2/tcgui/tc.netif.v1.toml");
    let (set, problems) = ContractSet::load_path(&path);
    assert!(problems.is_empty(), "{problems:?}");
    set
}

fn stamp(ntp64: u64, id: u8) -> zenoh::time::Timestamp {
    zenoh::time::Timestamp::new(
        zenoh::time::NTP64(ntp64),
        zenoh::time::TimestampId::try_from([id].as_slice()).expect("a one-byte id"),
    )
}

fn view(key: &str, at: Instant, ts: Option<zenoh::time::Timestamp>, delete: bool) -> SampleView {
    SampleView {
        key: key.into(),
        payload: zenoh::bytes::ZBytes::from(if delete { vec![] } else { vec![1u8, 2, 3] }),
        encoding: "application/octet-stream".into(),
        kind: if delete {
            zenoh::sample::SampleKind::Delete
        } else {
            zenoh::sample::SampleKind::Put
        },
        stamped_by: ts.map(|t| StampProvenance::Unattributable {
            stamper: *t.get_id(),
        }),
        timestamp: ts,
        attachment: None,
        priority: zenoh::qos::Priority::Data,
        congestion_control: zenoh::qos::CongestionControl::Drop,
        reliability: zenoh::qos::Reliability::BestEffort,
        express: false,
        source: None,
        received: at,
    }
}

/// A window with a reorder (A arrives first, B was stamped first), a
/// second stamper, an unstamped sample, a tombstone, and a drop between
/// the third and fourth samples — every case the projection distinguishes.
fn live_window(epoch: Instant) -> (Vec<Arrival>, ZrecHeader) {
    let at = |ms: u64| epoch + Duration::from_millis(ms);
    let a = "acme/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0";
    let b = "acme/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth1";
    let c = "acme/zk2/host-b/tc/tc.netif.v1/state/interfaces/default/eth0";
    let items = vec![
        Arrival::Sample(Box::new(view(a, at(10), Some(stamp(2_000, 0x33)), false))),
        Arrival::Sample(Box::new(view(b, at(20), Some(stamp(1_000, 0x33)), false))),
        Arrival::Sample(Box::new(view("plain/key", at(30), None, false))),
        Arrival::Dropped(5),
        Arrival::Sample(Box::new(view(c, at(40), Some(stamp(1_500, 0x44)), true))),
    ];
    let header = ZrecHeader {
        zrec: ZREC_VERSION,
        selectors: vec!["acme/zk2/**".into()],
        base: BASE.into(),
        captured_at: "2026-10-09T00:00:00Z".into(),
        excluded: Some(zenkey_fleet::zrec_excluded(&["acme/zk2/**".to_owned()])),
        preamble: None,
        pre_roll: None,
    };
    (items, header)
}

enum Arrival {
    /// Boxed: a `SampleView` is a few hundred bytes beside a `u64`, and the
    /// lint is right that the enum should not carry that everywhere.
    Sample(Box<SampleView>),
    Dropped(u64),
}

/// What `zenctl timeline` does with a live drain: rows from views, breaks
/// at the row count they interrupted.
fn from_live(
    items: &[Arrival],
    epoch: Instant,
    lens: &Lens<'_>,
) -> (Vec<TimelineRow>, Vec<PlacedBreak>) {
    let mut rows = Vec::new();
    let mut breaks = Vec::new();
    for item in items {
        match item {
            Arrival::Sample(v) => rows.push(TimelineRow::from_view(v, epoch, lens)),
            Arrival::Dropped(n) => breaks.push(PlacedBreak {
                after: rows.len(),
                lane: None,
                kind: Break::Dropped(*n),
            }),
        }
    }
    (rows, breaks)
}

/// The same drain, through the file: written at the live epoch, read back.
fn through_zrec(
    items: &[Arrival],
    header: &ZrecHeader,
    epoch: Instant,
    lens: &Lens<'_>,
) -> (Vec<TimelineRow>, Vec<PlacedBreak>) {
    let mut w = ZrecWriter::new_at(Vec::new(), header, epoch).expect("writer");
    for item in items {
        match item {
            Arrival::Sample(v) => w.write_sample(v).expect("row"),
            Arrival::Dropped(n) => w.write_dropped(*n).expect("drop"),
        }
    }
    let buf = w.finish().expect("finish");
    let mut r = ZrecReader::new(buf.as_slice()).expect("reader");
    let mut rows = Vec::new();
    let mut breaks = Vec::new();
    while let Some(item) = r.next() {
        let item: ZrecItem = item.expect("a well-formed line");
        match Ingested::from_zrec(&item, lens) {
            Ingested::Row(row) => rows.push(row),
            Ingested::Break(kind) => breaks.push(PlacedBreak {
                after: rows.len(),
                lane: None,
                kind,
            }),
            Ingested::Preamble { .. } | Ingested::Trigger { .. } => {}
        }
    }
    (rows, breaks)
}

#[test]
fn the_same_window_from_live_and_from_zrec_is_the_same_timeline_on_both_axes() {
    let epoch = Instant::now();
    let set = contracts();
    let lens = Lens::new(BASE, None, &set).offline(&set).held(set.len());
    let (items, header) = live_window(epoch);
    let (live_rows, live_breaks) = from_live(&items, epoch, &lens);
    let (zrec_rows, zrec_breaks) = through_zrec(&items, &header, epoch, &lens);

    // Row for row first, so a divergence names the field rather than the
    // report.
    assert_eq!(live_rows, zrec_rows);
    assert_eq!(live_breaks, zrec_breaks);

    let source = TimelineSource::Zrec {
        path: "bus.zrec".into(),
    };
    let live = Window {
        rows: live_rows,
        breaks: live_breaks,
        scopes: header.selectors.clone(),
        window_s: None,
        source: source.clone(),
        keys_evicted: 0,
        lens: lens.scope(),
    };
    let zrec = Window {
        rows: zrec_rows,
        breaks: zrec_breaks,
        ..live.clone()
    };
    for order in [Order::Arrival, Order::Hlc] {
        assert_eq!(timeline(&live, order), timeline(&zrec, order));
    }

    // And the window is the interesting one: the reorder shows, the
    // second stamper makes the claim a skewed one, the unstamped sample is
    // excluded on HLC and lane-d on arrival, the drop sits where it fell.
    let by_hlc = timeline(&live, Order::Hlc);
    assert_eq!(by_hlc.unstamped_excluded, 1);
    assert!(matches!(
        by_hlc.axis,
        zenkey_fleet::report::AxisLabel::Hlc {
            claim: zenkey_fleet::report::HlcClaim::SkewedWallClock { .. }
        }
    ));
    let by_arrival = timeline(&live, Order::Arrival);
    assert_eq!(by_arrival.dropped, 5);
    assert!(matches!(
        by_arrival.rows[3],
        zenkey_fleet::report::TimelineEntry::Break { pos: 3, n: 5, .. }
    ));
    assert_eq!(by_arrival.rows.len(), 5);
    assert_eq!(by_hlc.rows.len(), 3);
    // The lanes are zk2 resources, resolved through the contract held
    // offline: one stream of host-a, one state of host-b, and the foreign
    // key has no HLC, so it lives in the unstamped lane.
    let lanes: Vec<&LaneId> = by_arrival.lanes.iter().map(|l| &l.lane).collect();
    assert!(
        lanes.contains(&&LaneId::Resource {
            address: "host-a/tc".into(),
            iface: "tc.netif.v1".into(),
            token: "stream".into(),
            resource: Some("stream/bandwidth/{ns}/{iface}".into()),
        }),
        "{lanes:?}"
    );
    assert!(
        lanes.contains(&&LaneId::Resource {
            address: "host-b/tc".into(),
            iface: "tc.netif.v1".into(),
            token: "state".into(),
            resource: Some("state/interfaces/{ns}/{iface}".into()),
        }),
        "{lanes:?}"
    );
    assert!(lanes.contains(&&LaneId::Unstamped), "{lanes:?}");
    // A file names no owner: every stamp is unattributable, never foreign.
    assert!(
        by_arrival
            .lanes
            .iter()
            .all(|l| l.provenance.owner == 0 && l.provenance.other == 0)
    );
}

/// The `Dropped` variant of the live stream is the break the file records.
#[test]
fn a_stream_drop_and_a_file_drop_are_one_break() {
    let live = match StreamItem::Dropped(9) {
        StreamItem::Dropped(n) => Break::Dropped(n),
        StreamItem::Event(_) => unreachable!(),
    };
    let set = ContractSet::new();
    let lens = Lens::new(BASE, None, &set);
    assert_eq!(
        Ingested::from_zrec(&ZrecItem::Dropped(9), &lens),
        Ingested::Break(live)
    );
}

//! The same window, live and from the file, is the same timeline (#216).
//!
//! The acceptance criterion behind "nothing in `model/` takes a session":
//! a `.zrec` replays through the same projection as live traffic, and the
//! two reports are *equal* — not similar, not the same length. Rows are
//! built from `SampleView`s, written through `ZrecWriter::new_at` with the
//! same epoch the live rows measure from, read back with `ZrecReader`, and
//! projected both ways on both axes.

use std::time::{Duration, Instant};

use zenkey_fleet::report::TimelineSource;
use zenkey_fleet::{
    Break, Ingested, Order, PlacedBreak, SampleView, StampProvenance, StreamItem, TimelineRow,
    Window, ZREC_VERSION, ZrecHeader, ZrecItem, ZrecReader, ZrecWriter, timeline,
};

const BASE: &str = "acme";

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
    let a = "acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/a";
    let b = "acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/b";
    let c = "acme/v1/h-9a1b2c3d4e5f/state/logs/health";
    let items = vec![
        Arrival::Sample(Box::new(view(a, at(10), Some(stamp(2_000, 0x33)), false))),
        Arrival::Sample(Box::new(view(b, at(20), Some(stamp(1_000, 0x33)), false))),
        Arrival::Sample(Box::new(view("plain/key", at(30), None, false))),
        Arrival::Dropped(5),
        Arrival::Sample(Box::new(view(c, at(40), Some(stamp(1_500, 0x44)), true))),
    ];
    let header = ZrecHeader {
        zrec: ZREC_VERSION,
        selectors: vec!["acme/v1/**".into()],
        base: BASE.into(),
        captured_at: "2026-09-06T00:00:00Z".into(),
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
fn from_live(items: &[Arrival], epoch: Instant) -> (Vec<TimelineRow>, Vec<PlacedBreak>) {
    let mut rows = Vec::new();
    let mut breaks = Vec::new();
    for item in items {
        match item {
            Arrival::Sample(v) => rows.push(TimelineRow::from_view(v, epoch, BASE)),
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
        match Ingested::from_zrec(&item, BASE) {
            Ingested::Row(row) => rows.push(row),
            Ingested::Break(kind) => breaks.push(PlacedBreak {
                after: rows.len(),
                lane: None,
                kind,
            }),
        }
    }
    (rows, breaks)
}

#[test]
fn the_same_window_from_live_and_from_zrec_is_the_same_timeline_on_both_axes() {
    let epoch = Instant::now();
    let (items, header) = live_window(epoch);
    let (live_rows, live_breaks) = from_live(&items, epoch);
    let (zrec_rows, zrec_breaks) = through_zrec(&items, &header, epoch);

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
}

/// The `Dropped` variant of the live stream is the break the file records.
#[test]
fn a_stream_drop_and_a_file_drop_are_one_break() {
    let live = match StreamItem::Dropped(9) {
        StreamItem::Dropped(n) => Break::Dropped(n),
        StreamItem::Event(_) => unreachable!(),
    };
    assert_eq!(
        Ingested::from_zrec(&ZrecItem::Dropped(9), BASE),
        Ingested::Break(live)
    );
}

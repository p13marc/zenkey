//! `spec/profiles/telemetry/scenarios-v2.md` §1–§7 (`telemetry.v2`, #983
//! T3), one test per section, named after it, and the owner's meter beside
//! them.
//!
//! Common setup, the text's: R1 is a router, each test's own on an
//! ephemeral port. The owner O is `lab/meterframe`, implementing
//! `meterframe.v1` as the spec keeps it
//! (`spec/profiles/telemetry/conformance/contracts/meterframe.v1.toml`)
//! with every resource exposed, as a client of R1 whose session runs with
//! timestamping off (core §8.2). Its frames go through the runtime's frame
//! writers, which meter each record ([`zenkey::telemetry`]): it writes each
//! count unwrapped, and its frames carry it modulo its field's width. R
//! subscribes to `zk2/lab/meterframe/meterframe.v1/stream/**` before O
//! starts, and reads no presence; it keeps a
//! `zenkey_model::telemetry::frames::Fold` per frame key, handed each frame
//! in arrival order with its arrival on R's monotonic clock. "R reads" is
//! the fold's reading of each series field, and "R pairs" the fold's pair
//! of two consecutive samples of a counter series: every verdict is the
//! model's. A bare JSON counter R pairs by `telemetry.v1`'s rules
//! (`zenkey_model::telemetry::pair`), with no presence. Time is the
//! writers' `record_json_at` instants, so a section that the text spaces
//! 1 s apart writes its records 1 s apart on the owner's monotonic clock.
//!
//! A *raw* step puts a frame from O's session, under its run at its key's
//! next seq, after the writer refused the same record.

mod common;

#[allow(dead_code)]
#[path = "../examples/owner/config.rs"]
mod sim;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{client, config, eventually, router, router_with};
use serde_json::{Value, json};
use zenkey::frame::FrameWriter;
use zenkey::implementation::Implementation;
use zenkey::model::contract::{Body, Contract, Resource, load_path};
use zenkey::model::frame::{self as fm, Follows, Frame, Layout, Slot, Unknown};
use zenkey::model::grammar::IfaceId;
use zenkey::model::telemetry::frames::{
    FieldReading, FieldSeries, Fold, Folded, Position, Series, series_of,
};
use zenkey::model::telemetry::{
    Basis, Continuity, Measure, NoRate, Pair, Sampled, TimeBasis, Via, pair,
};
use zenkey::model::template::Bindings;
use zenkey::model::wire::{Envelope, Seq, Stamp};
use zenkey::run::FileRunStore;
use zenkey::telemetry::Reason;
use zenkey::writer::Writer;
use zenkey::{Error, Service, ServiceBuilder};
use zenoh::bytes::Encoding;
use zenoh::sample::Sample;

const SVC: &str = "lab/meterframe";
const DATA: &str = "zk2/lab/meterframe/meterframe.v1/stream/**";
const LINK: &str = "frames/link";
const TOTALS: &str = "frames/totals";
const BYTES: &str = "frames/bytes";
const DROPPED: &str = "sdu/dropped_total";
const IN_BAND: Option<Basis> = Some(Basis::Established(Via::InBand));

fn iface() -> IfaceId {
    "meterframe.v1".parse().unwrap()
}

/// `meterframe.v1`, the spec's copy.
fn meterframe() -> Contract {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spec/profiles/telemetry/conformance/contracts/meterframe.v1.toml");
    let l = load_path(&path);
    l.contract
        .unwrap_or_else(|| panic!("meterframe.v1 does not load:\n{}", l.report))
}

fn key_of(template: &str) -> String {
    format!("zk2/lab/meterframe/meterframe.v1/stream/{template}")
}

fn resource<'a>(c: &'a Contract, template: &str) -> &'a Resource {
    c.resources
        .iter()
        .find(|r| r.template.as_str() == template)
        .expect("a resource of meterframe.v1")
}

// ---------------------------------------------------------------------------
// O, the owner
// ---------------------------------------------------------------------------

/// O: its session, its service, a frame writer per frame resource, and the
/// bare counter's writer.
struct Owner {
    session: zenoh::Session,
    svc: Service,
    frames: BTreeMap<&'static str, FrameWriter>,
    dropped: Writer,
}

/// O started on a session of its own, as a client of the router at `ep`,
/// its runs from the run file `runs` if given.
async fn start(ep: &str, runs: Option<&Path>) -> Owner {
    let session = client(ep).await;
    let mut b = ServiceBuilder::new(&session, config(SVC));
    if let Some(p) = runs {
        b.run_store(Arc::new(FileRunStore::new(p))).unwrap();
    }
    owner_of(session, b).await
}

/// O on `session`, from `b`, which may implement more beside
/// `meterframe.v1`.
async fn owner_of(session: zenoh::Session, mut b: ServiceBuilder) -> Owner {
    b.implement(Implementation::new(meterframe())).unwrap();
    let mut frames = BTreeMap::new();
    for r in [LINK, TOTALS, BYTES] {
        let w = b
            .declare_frame_writer(&iface(), &format!("stream/{r}"))
            .await
            .unwrap();
        assert!(w.is_metered().await, "{r}: its series are declared");
        frames.insert(r, w);
    }
    let dropped = b
        .declare_writer(&iface(), &format!("stream/{DROPPED}"), &Bindings::new())
        .await
        .unwrap();
    let svc = b.start().await.unwrap();
    let o = Owner {
        session,
        svc,
        frames,
        dropped,
    };
    for w in o.frames.values() {
        eventually("R's subscription reaches O", || async {
            w.writer().matching().await.unwrap()
        })
        .await;
    }
    eventually("R's subscription reaches O", || async {
        o.dropped.matching().await.unwrap()
    })
    .await;
    o
}

impl Owner {
    fn w(&self, r: &str) -> &FrameWriter {
        &self.frames[r]
    }

    /// One record of `r`, measured at `at`: the stamp of the frame it put.
    async fn record(&self, r: &str, value: Value, at: Instant) -> Option<Stamp> {
        self.w(r).record_json_at(&value, at).await.unwrap()
    }

    /// `frames/totals`' one record, measured at `at`: its frame's stamp.
    async fn totals(&self, total: u64, temperature: &str, at: Instant) -> Stamp {
        self.record(
            TOTALS,
            json!({"frames_total": total, "temperature": temperature}),
            at,
        )
        .await
        .expect("one record a frame")
    }

    /// A *raw* frame of `r`: one full record of `value`, under `stamp`,
    /// from O's session; its time `at_ms` from the deployment epoch 0.
    async fn raw(&self, r: &str, value: &Value, stamp: Stamp, at_ms: i128) {
        let l = self.w(r).layout();
        let frame = Frame {
            time: fm::Time::from_unix_ms(at_ms, 0),
            records: vec![fm::Record {
                full: true,
                at: None,
                slots: l.slots_of_json(value, Some(0)).unwrap(),
            }],
        };
        self.session
            .put(key_of(r), frame.envelope(l, stamp).unwrap())
            .encoding(Encoding::ZENOH_BYTES)
            .await
            .unwrap();
    }

    /// The process ends: its session closes, and its tokens with it.
    async fn end(self) {
        drop(self.frames);
        self.session.close().await.unwrap();
        drop(self.svc);
    }
}

/// The meter's refusal of a record, as `telemetry.v2` reasons it.
fn refusal(e: Error) -> Reason {
    match e {
        Error::Telemetry { reason, .. } => *reason,
        other => panic!("not a telemetry refusal: {other}"),
    }
}

// ---------------------------------------------------------------------------
// R, the reader
// ---------------------------------------------------------------------------

/// A sample R heard: its key, payload, encoding and arrival.
#[derive(Clone)]
struct Heard {
    key: String,
    payload: Vec<u8>,
    enveloped: bool,
    at: Instant,
}

/// R: its subscriber, what it heard, and its clock's origin.
struct Reader {
    _sub: zenoh::pubsub::Subscriber<()>,
    heard: Arc<Mutex<Vec<Heard>>>,
    origin: Instant,
    contract: Contract,
}

async fn reader(s: &zenoh::Session) -> Reader {
    let heard: Arc<Mutex<Vec<Heard>>> = Arc::default();
    let h = Arc::clone(&heard);
    let sub = s
        .declare_subscriber(DATA)
        .callback(move |x: Sample| {
            h.lock().unwrap().push(Heard {
                key: x.key_expr().as_str().to_owned(),
                payload: x.payload().to_bytes().into_owned(),
                enveloped: *x.encoding() == Encoding::ZENOH_BYTES,
                at: Instant::now(),
            });
        })
        .await
        .unwrap();
    Reader {
        _sub: sub,
        heard,
        origin: Instant::now(),
        contract: meterframe(),
    }
}

/// One series field of one record, as R read it, with its pair.
#[derive(Debug, Clone)]
struct Read {
    at: Position,
    reading: FieldReading,
    pair: Option<(Position, Pair)>,
}

/// A frame key's fold: each frame as folded, and each series' readings.
struct Folding {
    series: Vec<FieldSeries>,
    frames: Vec<(Stamp, Frame, Folded)>,
}

impl Folding {
    fn series(&self, field: &str) -> &FieldSeries {
        self.series.iter().find(|s| s.field == field).unwrap()
    }

    /// Every record's reading of `field`, in order, at its position.
    fn reads(&self, field: &str) -> Vec<Read> {
        let k = self.series.iter().position(|s| s.field == field).unwrap();
        let mut out = Vec::new();
        for (n, (_, _, folded)) in self.frames.iter().enumerate() {
            for (record, r) in folded.records.iter().enumerate() {
                let x = r.fields.iter().find(|x| x.series == k).unwrap();
                out.push(Read {
                    at: Position {
                        frame: n as u64,
                        record,
                    },
                    reading: x.reading.clone(),
                    pair: x.pair.clone(),
                });
            }
        }
        out
    }

    /// A counter series' pairs: its two counts and the pair.
    fn pairs(&self, field: &str) -> Vec<(u64, u64, Pair)> {
        let reads = self.reads(field);
        let count = |at: Position| {
            let r = reads.iter().find(|r| r.at == at).unwrap();
            counter(r.reading.sample().expect("a sample").measure.clone())
        };
        reads
            .iter()
            .filter_map(|r| {
                let (before, p) = r.pair.clone()?;
                Some((count(before), count(r.at), p))
            })
            .collect()
    }

    /// The pair of `a` → `b`.
    fn pair(&self, field: &str, a: u64, b: u64) -> Pair {
        self.pairs(field)
            .into_iter()
            .find(|(x, y, _)| (*x, *y) == (a, b))
            .unwrap_or_else(|| panic!("{field}: no pair {a} → {b}: {:?}", self.pairs(field)))
            .2
    }
}

fn counter(m: Measure) -> u64 {
    match m {
        Measure::Counter { value, .. } => value,
        m => panic!("not a counter: {m:?}"),
    }
}

impl Reader {
    /// Waits for `n` samples on `template`'s key.
    async fn wait(&self, template: &str, n: usize) {
        let key = key_of(template);
        eventually(&format!("{n} samples of {template}"), || async {
            self.heard
                .lock()
                .unwrap()
                .iter()
                .filter(|h| h.key == key)
                .count()
                >= n
        })
        .await;
    }

    fn of(&self, template: &str) -> Vec<Heard> {
        let key = key_of(template);
        self.heard
            .lock()
            .unwrap()
            .iter()
            .filter(|h| h.key == key)
            .cloned()
            .collect()
    }

    /// `template`'s frames, folded in arrival order (v2 §2.18–§2.21).
    fn fold(&self, template: &str) -> Folding {
        let r = resource(&self.contract, template);
        let layout = Layout::of_resource(&self.contract, r).unwrap().unwrap();
        let Some(Series::Declared(series)) = series_of(&self.contract, r) else {
            panic!("{template}: its series are declared");
        };
        let mut fold = Fold::new();
        let frames = self
            .of(template)
            .into_iter()
            .map(|h| {
                let e = Envelope::parse(&h.payload).unwrap();
                assert!(e.is_frame(), "{template}: a frame");
                let frame = Frame::parse(e.tag, e.record, &layout).unwrap();
                let arrival = i128::try_from(h.at.duration_since(self.origin).as_nanos()).unwrap();
                let folded = fold.apply(&layout, &series, e.stamp, &frame, arrival);
                (e.stamp, frame, folded)
            })
            .collect();
        Folding { series, frames }
    }

    /// The bare counter's samples, as `telemetry.v1` reads them: each
    /// count and its arrival.
    fn bare(&self) -> Vec<(Measure, i128)> {
        self.of(DROPPED)
            .into_iter()
            .map(|h| {
                let bytes = if h.enveloped {
                    Envelope::parse(&h.payload).unwrap().record.to_vec()
                } else {
                    h.payload.clone()
                };
                let n: u64 = serde_json::from_slice(&bytes).unwrap();
                let at = i128::try_from(h.at.duration_since(self.origin).as_nanos()).unwrap();
                (Measure::counter(n), at)
            })
            .collect()
    }

    /// The bare counter's consecutive pairs, by v1's rules with no
    /// presence: no epoch rides it, so each is assumed (v1 §2.9).
    fn bare_pairs(&self) -> Vec<(u64, u64, Pair)> {
        let s = self.bare();
        fn sampled((m, at): &(Measure, i128)) -> Sampled<'_> {
            Sampled {
                value: m,
                epoch: None,
                timestamp_ms: None,
                stamp_ns: None,
                arrival_ns: *at,
                record: None,
            }
        }
        s.windows(2)
            .map(|w| {
                (
                    counter(w[0].0.clone()),
                    counter(w[1].0.clone()),
                    pair(&sampled(&w[0]), &sampled(&w[1]), Some("1"), None),
                )
            })
            .collect()
    }
}

/// An elapsed time within the scenarios' loopback jitter of `ms`.
fn about(elapsed_ns: i128, ms: i128) {
    assert!(
        (elapsed_ns - ms * 1_000_000).abs() <= 50_000_000,
        "{elapsed_ns} ns, not about {ms} ms"
    );
}

/// v1 §2.11's formula, which a pair's rate keeps.
fn rate(d: u64, elapsed_ns: i128) -> f64 {
    d as f64 / (elapsed_ns as f64 / 1e9)
}

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

// ---------------------------------------------------------------------------
// The scenarios
// ---------------------------------------------------------------------------

/// §1: a counter's rate over frames, measured and scheduled (§2.18,
/// §2.21).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_a_counters_rate_over_frames_measured_and_scheduled() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;
    let o = start(&ep, None).await;

    // 1. Two link frames, 2 s apart.
    let t = Instant::now();
    for (n, dropped) in [(0, 10), (2, 20)] {
        let link = json!({"dropped": dropped, "queue_depth": 3, "duty_available": 1,
                          "duty_wait_ms": "0"});
        o.record(LINK, link, t + s(n))
            .await
            .expect("one record a frame");
    }
    // 2. rx 1000, 1500, 2100, 1 s apart: one frame, records 1 and 2 scheduled.
    let b = t + s(10);
    assert_eq!(o.record(BYTES, json!({"rx": 1000}), b).await, None);
    assert_eq!(o.record(BYTES, json!({"rx": 1500}), b + s(1)).await, None);
    let first = o.record(BYTES, json!({"rx": 2100}), b + s(2)).await;
    assert!(first.is_some(), "three records: a frame");
    // 3. rx 2600 a second later, the next frame's record 0, put.
    assert_eq!(o.record(BYTES, json!({"rx": 2600}), b + s(3)).await, None);
    assert!(o.w(BYTES).flush().await.unwrap().is_some());
    r.wait(LINK, 2).await;
    r.wait(BYTES, 2).await;

    // 1. Every series field of frames/link a sample; dropped 10 → 20.
    let link = r.fold(LINK);
    for f in ["dropped", "queue_depth", "duty_available", "duty_wait_ms"] {
        for x in link.reads(f) {
            assert_eq!(x.reading.as_str(), "sample", "{f}: {x:?}");
        }
    }
    let p = link.pair("dropped", 10, 20);
    assert_eq!((p.continuity, p.basis), (Continuity::SameEpoch, IN_BAND));
    let rt = p.rate.unwrap();
    about(rt.elapsed_ns, 2_000);
    assert_eq!(rt.per_s, rate(10, rt.elapsed_ns));
    assert!((rt.per_s - 5.0).abs() < 0.2, "{}", rt.per_s);
    assert_eq!(
        (rt.unit.as_deref(), rt.basis),
        (Some("1/s"), Some(TimeBasis::Measured))
    );

    // 2. Within the frame, by the records' offsets: exact, scheduled.
    let bytes = r.fold(BYTES);
    let p = bytes.pair("rx", 1000, 1500);
    assert_eq!(p.continuity, Continuity::SameEpoch);
    let rt = p.rate.unwrap();
    assert_eq!((rt.per_s, rt.elapsed_ns), (500.0, 1_000_000_000));
    assert_eq!(rt.basis, Some(TimeBasis::Scheduled { jitter_ms: 100 }));
    assert_eq!(rt.unit.as_deref(), Some("By/s"));
    let rt = bytes.pair("rx", 1500, 2100).rate.unwrap();
    assert_eq!((rt.per_s, rt.elapsed_ns), (600.0, 1_000_000_000));
    assert_eq!(rt.basis, Some(TimeBasis::Scheduled { jitter_ms: 200 }));
    // 3. Across the two frames, by their instants.
    let rt = bytes.pair("rx", 2100, 2600).rate.unwrap();
    about(rt.elapsed_ns, 1_000);
    assert_eq!(rt.per_s, rate(500, rt.elapsed_ns));
    assert_eq!(rt.basis, Some(TimeBasis::Scheduled { jitter_ms: 100 }));
    // 4. No scheduled rate is a measured one.
    for (_, _, p) in bytes.pairs("rx") {
        let basis = p.rate.unwrap().basis;
        assert!(
            matches!(basis, Some(TimeBasis::Scheduled { .. })),
            "{basis:?}"
        );
    }
    // 5. The gauges: samples, and no pair.
    for f in ["queue_depth", "duty_available", "duty_wait_ms"] {
        assert!(link.reads(f).iter().all(|x| x.pair.is_none()), "{f}");
    }
    o.end().await;
}

/// §2: a restart, a new run, a reset (§2.19).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_a_restart_a_new_run_a_reset() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;

    // 1. I1 under r1: 100, then 140.
    let o = start(&ep, None).await;
    let t = Instant::now();
    let r1 = o.totals(100, "21.5", t).await.run;
    o.totals(140, "21.5", t + s(1)).await;
    r.wait(TOTALS, 2).await;
    // 2. The process ends; I2 under r2, its count from 0 again: 3, then 9.
    o.end().await;
    let o = start(&ep, None).await;
    let t = Instant::now();
    let r2 = o.totals(3, "21.5", t).await;
    assert_ne!(r2.run, r1, "a new instance takes a new run (core §4.3)");
    assert_eq!(r2.seq.get(), 0);
    o.totals(9, "21.5", t + s(1)).await;
    r.wait(TOTALS, 4).await;

    let f = r.fold(TOTALS);
    // 1. 100 → 140 within r1.
    let p = f.pair("frames_total", 100, 140);
    assert_eq!((p.continuity, p.basis), (Continuity::SameEpoch, IN_BAND));
    assert!(p.rate.is_ok());
    // 2. I2's first frame is another run's, a keyframe: each field known.
    let (_, frame, folded) = &f.frames[2];
    assert_eq!(folded.applied.follows, Follows::Run);
    assert!(frame.records[0].full);
    for x in [f.reads("frames_total"), f.reads("temperature")] {
        assert_eq!(x[2].reading.as_str(), "sample", "{x:?}");
    }
    // 3. 140 → 3: a reset, in band, no rate, no finding.
    let p = f.pair("frames_total", 140, 3);
    assert_eq!((p.continuity, p.basis), (Continuity::Reset, IN_BAND));
    assert_eq!(p.rate, Err(NoRate::Reset));
    // 4. 3 → 9 within r2.
    let p = f.pair("frames_total", 3, 9);
    assert_eq!((p.continuity, p.basis), (Continuity::SameEpoch, IN_BAND));
    assert!(p.rate.is_ok());
    o.end().await;
}

/// §3: a 24-bit counter's wrap (§2.20).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_a_24_bit_counters_wrap() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;
    let o = start(&ep, None).await;

    // 1. Three counts in one frame, 1 s apart, the last past §2.20's SHOULD:
    // the meter writes it, a warning and no refusal.
    let t = Instant::now();
    assert_eq!(o.record(BYTES, json!({"rx": 16_777_000}), t).await, None);
    assert_eq!(
        o.record(BYTES, json!({"rx": 16_777_316}), t + s(1)).await,
        None
    );
    let put = o.record(BYTES, json!({"rx": 25_777_316}), t + s(2)).await;
    assert!(put.is_some());
    r.wait(BYTES, 1).await;

    let f = r.fold(BYTES);
    // 1. Each count modulo 2^24.
    let slots: Vec<Slot> = f.frames[0].1.records.iter().map(|x| x.slots[0]).collect();
    assert_eq!(
        slots,
        [
            Slot::Code(16_777_000),
            Slot::Code(100),
            Slot::Code(9_000_100)
        ]
    );
    // 2. 16,777,000 → 100: the wrap included, 316 By/s.
    let p = f.pair("rx", 16_777_000, 100);
    assert_eq!((p.continuity, p.basis), (Continuity::SameEpoch, IN_BAND));
    let rt = p.rate.unwrap();
    assert_eq!(rt.per_s, 316.0);
    assert!(matches!(rt.basis, Some(TimeBasis::Scheduled { .. })));
    // 3. 100 → 9,000,100: past the window, unobservable, no rate.
    let p = f.pair("rx", 100, 9_000_100);
    assert_eq!(p.continuity, Continuity::Unobservable);
    assert_eq!(p.unobservable(), Some("wrap_or_decrease"));
    assert_eq!(p.rate, Err(NoRate::WrapOrDecrease));
    o.end().await;
}

/// §4: a lost frame, unknown, never zero (§2.18).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_a_lost_frame_unknown_never_zero() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;
    let o = start(&ep, None).await;

    // 1. Four frames 1 s apart, the third lost; 2. a fifth, a keyframe.
    let t = Instant::now();
    o.totals(10, "20", t).await;
    o.totals(10, "20.5", t + s(1)).await;
    o.w(TOTALS).simulate_loss();
    let lost = o.totals(15, "20.5", t + s(2)).await;
    o.totals(15, "21", t + s(3)).await;
    o.totals(18, "21", t + s(4)).await;
    assert_eq!(lost.seq.get(), 2);
    r.wait(TOTALS, 4).await;

    let f = r.fold(TOTALS);
    let seqs: Vec<u32> = f.frames.iter().map(|(s, _, _)| s.seq.get()).collect();
    assert_eq!(seqs, [0, 1, 3, 4], "seq 2 taken, never put");
    let total = f.reads("frames_total");
    let temperature = f.reads("temperature");
    // 1. Record 2's frames_total omitted: held, known 10, no sample.
    assert_eq!(f.frames[1].1.records[0].slots[0], Slot::Omitted);
    match &total[1].reading {
        FieldReading::Held { value, within } => {
            assert_eq!((counter(value.measure.clone()), *within), (10, false));
        }
        other => panic!("held, not {other:?}"),
    }
    // 2. Record 4 after a gap of one record: unknown, never 10, never 0.
    assert_eq!(f.frames[2].2.applied.follows, Follows::Gap { lost: 1 });
    assert_eq!(total[2].reading, FieldReading::Unknown(Unknown::Gap));
    assert_eq!(total[2].reading.value(), None, "no value, neither 10 nor 0");
    assert!(total[2].pair.is_none());
    assert_eq!(temperature[2].reading.as_str(), "sample");
    // 3. Record 5, a keyframe: 18, paired with record 1's 10.
    assert!(f.frames[3].1.records[0].full);
    let p = f.pair("frames_total", 10, 18);
    assert_eq!((p.continuity, p.basis), (Continuity::SameEpoch, IN_BAND));
    let rt = p.rate.unwrap();
    about(rt.elapsed_ns, 4_000);
    assert_eq!(rt.per_s, rate(8, rt.elapsed_ns));
    assert!((rt.per_s - 2.0).abs() < 0.05, "{}", rt.per_s);
    assert_eq!(rt.basis, Some(TimeBasis::Measured));
    assert_eq!(
        f.pairs("frames_total").len(),
        1,
        "no pair through the gap but this one"
    );
    o.end().await;
}

/// §5: a 64-bit counter's decrease across a face is the finding (§2.19;
/// v1 §2.14). The face is R1's policy, which denies `@zk` (`link.v1`
/// §2.6): R reads no presence, and there is none to read.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_64_bit_counters_decrease_across_a_face_is_the_finding() {
    let acl = r#"{
        enabled: true,
        default_permission: "allow",
        rules: [{
            id: "zk",
            messages: ["liveliness_token", "liveliness_query", "declare_liveliness_subscriber",
                       "query", "reply", "declare_queryable"],
            flows: ["ingress", "egress"],
            permission: "deny",
            key_exprs: ["zk2/*/*/@zk/**"],
        }],
        subjects: [{ id: "anyone" }],
        policies: [{ rules: ["zk"], subjects: ["anyone"] }],
    }"#;
    let (_r1, ep) = router_with(None, &[("access_control", acl)]).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;
    let o = start(&ep, None).await;
    let tokens = zenkey::presence::liveliness_read(&rs, "zk2/lab/meterframe/@zk/**", common::T)
        .await
        .unwrap();
    assert!(tokens.keys.is_empty(), "no presence crosses: {tokens:?}");

    // 1. 500, through the writer.
    let t = Instant::now();
    let first = o.totals(500, "21.5", t).await;
    // 2. 480: the runtime refuses it, then the raw frame.
    let e = o
        .w(TOTALS)
        .record_json_at(
            &json!({"frames_total": 480, "temperature": "21.5"}),
            t + s(1),
        )
        .await
        .unwrap_err();
    assert!(e.to_string().starts_with("telemetry.v2: "), "{e}");
    assert_eq!(
        refusal(e),
        Reason::FieldDecreased {
            field: "frames_total".into(),
            before: 500,
            after: 480,
        }
    );
    assert_eq!(o.w(TOTALS).open().await, 0, "the record joined no frame");
    let next = Stamp {
        run: first.run,
        seq: Seq::new(first.seq.get() + 1).unwrap(),
    };
    let now = i128::from(zenkey::telemetry::now_ms());
    o.raw(
        TOTALS,
        &json!({"frames_total": 480, "temperature": "21.5"}),
        next,
        now,
    )
    .await;
    // 3. The bare counter: 500 through its writer, 480 raw, unenveloped.
    o.dropped.put_value(&500u64).await.unwrap();
    let e = o.dropped.put_value(&480u64).await.unwrap_err();
    assert!(e.to_string().starts_with("telemetry.v1: "), "{e}");
    o.session
        .put(key_of(DROPPED), "480")
        .encoding(Encoding::APPLICATION_JSON)
        .await
        .unwrap();
    r.wait(TOTALS, 2).await;
    r.wait(DROPPED, 2).await;

    // 1. 500 → 480 within r1: decreased, in band, with no presence.
    let f = r.fold(TOTALS);
    let p = f.pair("frames_total", 500, 480);
    assert_eq!((p.continuity, p.basis), (Continuity::Decreased, IN_BAND));
    assert_eq!(p.rate, Err(NoRate::Decreased));
    // 3. The bare counter's decrease: a reset, assumed (v1 §2.14).
    let bare = r.bare_pairs();
    assert_eq!(bare.len(), 1, "{bare:?}");
    let (a, b, p) = &bare[0];
    assert_eq!((*a, *b), (500, 480));
    assert_eq!(
        (p.continuity, p.basis),
        (Continuity::Reset, Some(Basis::Assumed))
    );
    o.end().await;
}

/// §6: a rewind under a restored run file is a reset (§2.19).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_a_rewind_under_a_restored_run_file_is_a_reset() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;
    let dir = tempfile::tempdir().unwrap();
    let runs = dir.path().join("runs");
    std::fs::write(&runs, 6u16.to_be_bytes()).unwrap();
    let copy = std::fs::read(&runs).unwrap();

    // 1. I1 under run r: 1000, then 1010.
    let o = start(&ep, Some(&runs)).await;
    let t = Instant::now();
    let a = o.totals(1000, "21.5", t).await;
    o.totals(1010, "21.5", t + s(1)).await;
    r.wait(TOTALS, 2).await;
    // 2. The process ends, and its run file is restored.
    o.end().await;
    std::fs::write(&runs, &copy).unwrap();
    // 3. I2 takes r again, and its count starts again: 4 at seq 0.
    let o = start(&ep, Some(&runs)).await;
    let b = o.totals(4, "21.5", Instant::now()).await;
    assert_eq!(
        (b.run, b.seq.get()),
        (a.run, 0),
        "r again, which §4.3 forbids"
    );
    r.wait(TOTALS, 3).await;

    let f = r.fold(TOTALS);
    // 1. A seq of the run R holds, older than it expected: a rewind, its
    // keyframe making each field known again.
    let (_, frame, folded) = &f.frames[2];
    assert_eq!(folded.applied.follows, Follows::Rewind);
    assert!(frame.records[0].full);
    assert_eq!(f.reads("frames_total")[2].reading.as_str(), "sample");
    // 2. 1010 → 4: a reset, in band, never decreased.
    let p = f.pair("frames_total", 1010, 4);
    assert_eq!((p.continuity, p.basis), (Continuity::Reset, IN_BAND));
    assert_eq!(p.rate, Err(NoRate::Reset));
    o.end().await;
}

/// §7: one counter, two series (§2.17).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_one_counter_two_series() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;
    let o = start(&ep, None).await;

    // 1. 7, on the bare stream and in frames/link, at one moment; 2. 9, 1 s
    // later.
    let t = Instant::now();
    for (n, v) in [(0, 7u64), (1, 9)] {
        if n > 0 {
            tokio::time::sleep(s(1)).await;
        }
        o.dropped.put_value(&v).await.unwrap();
        let link = json!({"dropped": v, "queue_depth": 3, "duty_available": 1,
                          "duty_wait_ms": "0"});
        o.record(LINK, link, t + s(n)).await.unwrap();
    }
    r.wait(LINK, 2).await;
    r.wait(DROPPED, 2).await;

    // 1. Two series: the bare key, and dropped of frames/link, its mirror
    // a label. 2. The mirror's kind and unit.
    let f = r.fold(LINK);
    let s = f.series("dropped");
    assert_eq!(s.mirror.as_deref(), Some(DROPPED));
    assert_eq!(
        (s.kind, s.unit.as_deref(), s.width),
        (
            zenkey::model::telemetry::MetricKind::Counter,
            Some("1"),
            Some(32)
        )
    );
    // 3. Each pair is its own series': the bare one assumed, the frame's in
    // band.
    let bare = r.bare_pairs();
    assert_eq!(bare.len(), 1);
    assert_eq!((bare[0].0, bare[0].1), (7, 9));
    assert_eq!(
        (bare[0].2.continuity, bare[0].2.basis),
        (Continuity::SameEpoch, Some(Basis::Assumed))
    );
    let pairs = f.pairs("dropped");
    assert_eq!(pairs.len(), 1);
    let p = &pairs[0].2;
    assert_eq!((pairs[0].0, pairs[0].1), (7, 9));
    assert_eq!((p.continuity, p.basis), (Continuity::SameEpoch, IN_BAND));
    // 4. frames/link's record type is core §7.7's fixtures' Link: its
    // layout is `layouts.json`'s `link`, and its bodies `bodies.json`'s.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/conformance/frame");
    let read = |f: &str| -> Value {
        serde_json::from_slice(&std::fs::read(root.join(f)).unwrap()).unwrap()
    };
    let layout = o.w(LINK).layout();
    let link = read("layouts.json")["layouts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["name"] == "link")
        .unwrap()
        .clone();
    assert_eq!(layout.form(), &link["expect"]["form"]);
    assert_eq!(
        u64::from(layout.tag().get()),
        link["expect"]["tag"].as_u64().unwrap()
    );
    let mut written = 0;
    for case in read("bodies.json")["writes"].as_array().unwrap() {
        if case["layout"] != "link" || case["expect"]["hex"].is_null() {
            continue;
        }
        let frame = Frame::from_json(&case["value"], layout, None).unwrap();
        let hex: String = frame
            .write(layout)
            .unwrap()
            .bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(hex, case["expect"]["hex"], "{}", case["note"]);
        written += 1;
    }
    assert!(written > 0, "bodies.json writes link frames");
    o.end().await;
}

// ---------------------------------------------------------------------------
// The owner's meter (v2 §2.18–§2.20), beside the scenarios
// ---------------------------------------------------------------------------

/// A restarted counter takes a new run (§2.19): a count below the last is
/// refused within the run, and written under the next. A record still in
/// the open frame is of the run that frame takes, so a restart flushes
/// first.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restarted_counter_takes_a_new_run() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let _r = reader(&rs).await;
    let mut o = start(&ep, None).await;
    let t = Instant::now();
    let a = o.totals(100, "21.5", t).await;
    let below = json!({"frames_total": 50, "temperature": "21.5"});
    let e = o.w(TOTALS).record_json_at(&below, t + s(1)).await;
    assert!(matches!(
        refusal(e.unwrap_err()),
        Reason::FieldDecreased { .. }
    ));
    o.svc.new_epoch().await.unwrap();
    let b = o.totals(50, "21.5", t + s(2)).await;
    assert_ne!(b.run, a.run, "a new run, compared with nothing");

    // An open frame across a new run: its records are put under the new
    // one, so the restarted count below them is refused.
    let bytes = o.w(BYTES);
    assert_eq!(
        bytes.record_json_at(&json!({"rx": 100}), t).await.unwrap(),
        None
    );
    o.svc.new_epoch().await.unwrap();
    let bytes = o.w(BYTES);
    let e = bytes.record_json_at(&json!({"rx": 5}), t + s(1)).await;
    assert_eq!(
        refusal(e.unwrap_err()),
        Reason::FieldDecreased {
            field: "rx".into(),
            before: 100,
            after: 5,
        }
    );
    // Flushed first, then the new run: the restarted count is written.
    bytes.flush().await.unwrap();
    o.svc.new_epoch().await.unwrap();
    let bytes = o.w(BYTES);
    assert_eq!(
        bytes
            .record_json_at(&json!({"rx": 5}), t + s(2))
            .await
            .unwrap(),
        None
    );
    assert!(bytes.flush().await.unwrap().is_some());
    o.end().await;
}

/// #1055: a `config.v1` owner whose device restarts, its counters with
/// it, takes a new run within its instance ([`Service::new_run`]), since a
/// new instance would restart its configuration (`config.v1` §2.12) and is
/// refused. R pairs each counter across as a reset, in band, never a
/// decrease (§2.19). The configuration's view, its pending change's token
/// and the instance's tokens stand, and the next put of the view goes
/// under the new run.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_config_v1_owners_device_restart_takes_a_new_run() {
    use zenkey::config_v1::{self, ConfigView};
    use zenkey::{CallMetadata, Client, Outcome};

    /// A call of `config.v1`'s `resource` at O, as `ops`: its value reply.
    async fn call(s: &zenoh::Session, resource: &str, group: Option<&str>, body: Value) -> Value {
        let c = Client::new(s, config_v1::implementation().shared_contract(), &[SVC])
            .unwrap()
            .with_timeout(Duration::from_secs(2))
            .with_metadata(CallMetadata::new("ops", resource));
        let values: Bindings = group
            .map(|g| [("group".to_owned(), vec![g.to_owned()])].into())
            .unwrap_or_default();
        match c
            .call_value(&SVC.parse().unwrap(), resource, &values, &body)
            .await
            .unwrap()
        {
            Outcome::Value(a) => a.value::<Value>().unwrap(),
            other => panic!("a value reply, not {other:?}"),
        }
    }

    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;
    // What R hears of O's tokens, and of its configuration's state with
    // the run each put is under.
    let heard: Arc<Mutex<Vec<String>>> = Arc::default();
    let (h1, h2) = (Arc::clone(&heard), Arc::clone(&heard));
    let _tokens = rs
        .liveliness()
        .declare_subscriber("zk2/lab/meterframe/@zk/**")
        .callback(move |x: Sample| {
            let line = format!("{:?} {}", x.kind(), x.key_expr());
            h1.lock().unwrap().push(line);
        })
        .await
        .unwrap();
    let _state = rs
        .declare_subscriber("zk2/lab/meterframe/config.v1/state/**")
        .callback(move |x: Sample| {
            let bytes = x.payload().to_bytes();
            let run = Envelope::parse(&bytes).map(|e| e.stamp.run.hex());
            let line = format!("{:?} {} run={run:?}", x.kind(), x.key_expr());
            h2.lock().unwrap().push(line);
        })
        .await
        .unwrap();
    let since = |mark: usize| heard.lock().unwrap()[mark..].to_vec();

    // O: meterframe.v1's frames, and config.v1 on the owner example's
    // simulated device.
    let session = client(&ep).await;
    let mut b = ServiceBuilder::new(&session, config(SVC));
    let startup = sim::rf_startup(false, 10, Duration::from_secs(1), false);
    let device = Arc::new(sim::SimDevice::new(&startup));
    let cfg = b
        .config_v1(startup, Arc::clone(&device) as Arc<dyn config_v1::Device>)
        .await
        .unwrap();
    let mut o = owner_of(session, b).await;
    let instance = o.svc.instance().to_string();
    eventually("R hears O's instance token", || async {
        since(0).iter().any(|h| h.contains("/@zk/instance/"))
    })
    .await;
    let duty = || device.holds("duty", "fraction_ppm");

    // 1. A pending change of the hot group `duty` (config.v1 §2.6).
    let set = call(
        &rs,
        config_v1::SET,
        Some("duty"),
        json!({"values": {"fraction_ppm": 20000}, "confirm_s": 60}),
    )
    .await;
    let view: ConfigView = serde_json::from_value(set["applied"].clone()).unwrap();
    let token = view.pending.expect("pending").token;
    assert_eq!(duty(), Some(config_v1::Value::Integer(20000)));
    eventually(
        "R hears the view and the status, at start and pending",
        || async { since(0).iter().filter(|h| h.contains("/state/")).count() >= 4 },
    )
    .await;

    // 2. Within the run r0: 100, then 120; rx 100 in the open frame.
    let t = Instant::now();
    let r0 = o.totals(100, "21.5", t).await.run;
    o.totals(120, "21.5", t + s(1)).await;
    assert_eq!(o.record(BYTES, json!({"rx": 100}), t).await, None);

    // 3. The device restarts, its counters from 0: below the last within
    // r0, refused; a new instance, a restart of the configuration, refused.
    let below = json!({"frames_total": 5, "temperature": "21.5"});
    let e = o.w(TOTALS).record_json_at(&below, t + s(2)).await;
    assert!(matches!(
        refusal(e.unwrap_err()),
        Reason::FieldDecreased { .. }
    ));
    let e = o.svc.new_epoch().await.expect_err("config.v1 §2.12");
    assert!(e.to_string().contains("config.v1 §2.12"), "{e}");

    // 4. The open frame flushed under r0, then a new run, r1, in the same
    // instance: the descriptor states it (core §3.3).
    assert_eq!(o.w(BYTES).flush().await.unwrap().expect("rx 100").run, r0);
    let view0 = cfg.view().await;
    let mark = since(0).len();
    let r1 = o.svc.new_run();
    assert_ne!(r1, r0, "a new run");
    assert_eq!(o.svc.minter().run(), r1);
    let d: Value = serde_json::from_slice(&o.svc.descriptor_bytes()).unwrap();
    assert_eq!(d["run"], r1.hex(), "the descriptor states it");
    assert_eq!(o.svc.instance().to_string(), instance, "no new instance");

    // 5. The restarted counts, under r1.
    let b = o.totals(5, "21.5", t + s(2)).await;
    assert_eq!((b.run, b.seq.get()), (r1, 0), "the key's seqs from 0");
    o.totals(9, "21.5", t + s(3)).await;
    assert_eq!(o.record(BYTES, json!({"rx": 5}), t + s(2)).await, None);
    assert_eq!(o.w(BYTES).flush().await.unwrap().expect("rx 5").run, r1);
    r.wait(TOTALS, 4).await;
    r.wait(BYTES, 2).await;

    // R: 100 → 120 within r0; r1's first frame another run's, a keyframe;
    // 120 → 5 a reset, in band, with no rate; 5 → 9 within r1; no pair a
    // decrease, on either key.
    let f = r.fold(TOTALS);
    let p = f.pair("frames_total", 100, 120);
    assert_eq!((p.continuity, p.basis), (Continuity::SameEpoch, IN_BAND));
    let (_, frame, folded) = &f.frames[2];
    assert_eq!(folded.applied.follows, Follows::Run);
    assert!(frame.records[0].full);
    let p = f.pair("frames_total", 120, 5);
    assert_eq!((p.continuity, p.basis), (Continuity::Reset, IN_BAND));
    assert_eq!(p.rate, Err(NoRate::Reset));
    let p = f.pair("frames_total", 5, 9);
    assert_eq!((p.continuity, p.basis), (Continuity::SameEpoch, IN_BAND));
    let bytes = r.fold(BYTES);
    let p = bytes.pair("rx", 100, 5);
    assert_eq!((p.continuity, p.basis), (Continuity::Reset, IN_BAND));
    for (field, x) in [("frames_total", &f), ("rx", &bytes)] {
        assert!(
            x.pairs(field)
                .iter()
                .all(|(_, _, p)| p.continuity != Continuity::Decreased),
            "{field}: {:?}",
            x.pairs(field)
        );
    }

    // 6. The configuration stands: its view, the device's values, and
    // every token of the instance; nothing of its state put again.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(cfg.view().await, view0);
    assert_eq!(duty(), Some(config_v1::Value::Integer(20000)));
    assert_eq!(since(mark), Vec::<String>::new(), "nothing moved");

    // 7. The pending change's token still names it: confirmed, and the
    // view put after it is under r1.
    let ok = call(&rs, config_v1::CONFIRM, None, json!({"token": token})).await;
    let view: ConfigView = serde_json::from_value(ok["view"].clone()).unwrap();
    assert_eq!(view.pending, None);
    assert_eq!(view.last_change.map(|l| l.token), Some(token));
    assert_eq!((view.instance, view.revision), (instance, view0.revision));
    let want = format!(
        "Put zk2/lab/meterframe/config.v1/state/view run=Ok({:?})",
        r1.hex()
    );
    eventually("R hears the view put under r1", || async {
        since(mark).contains(&want)
    })
    .await;
    o.end().await;
}

/// A record that lies past the open frame's span opens the next frame
/// (core §7.7): the open frame is put first, under the run then, and the
/// record is metered after it, so its count is of the run its own frame
/// takes. Across a new run before that frame is put, a count below it is
/// refused: it is the new run's, never the run of the frame put before it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_record_past_the_span_opens_the_next_frame() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;
    let mut o = start(&ep, None).await;
    let t = Instant::now();
    // frames/bytes: three records a frame, 1 s apart, so a span of 3 s.
    assert_eq!(o.record(BYTES, json!({"rx": 10}), t).await, None);
    assert_eq!(o.record(BYTES, json!({"rx": 20}), t + s(1)).await, None);
    let first = o
        .record(BYTES, json!({"rx": 30}), t + s(5))
        .await
        .expect("past the span: the open frame put first");
    assert_eq!(
        o.w(BYTES).open().await,
        1,
        "the record opens the next frame"
    );
    r.wait(BYTES, 1).await;
    let f = r.fold(BYTES);
    assert_eq!(f.frames.len(), 1);
    assert_eq!(f.frames[0].0, first);
    assert_eq!(f.frames[0].1.records.len(), 2, "10 and 20, not 30");
    // A new run while 30 is in the open frame: 30 is the new run's.
    o.svc.new_epoch().await.unwrap();
    let e = o.w(BYTES).record_json_at(&json!({"rx": 5}), t + s(6)).await;
    assert_eq!(
        refusal(e.unwrap_err()),
        Reason::FieldDecreased {
            field: "rx".into(),
            before: 30,
            after: 5,
        }
    );
    assert_eq!(o.record(BYTES, json!({"rx": 40}), t + s(6)).await, None);
    let second = o.w(BYTES).flush().await.unwrap().expect("30 and 40");
    assert_ne!(second.run, first.run, "put under the new run");
    r.wait(BYTES, 2).await;
    let f = r.fold(BYTES);
    assert_eq!(f.frames[1].1.records.len(), 2);
    assert_eq!(
        f.pairs("rx")
            .iter()
            .map(|(a, b, _)| (*a, *b))
            .collect::<Vec<_>>(),
        [(10, 20), (20, 30), (30, 40)]
    );
    assert_eq!(
        f.pair("rx", 20, 30).continuity,
        Continuity::Reset,
        "across the run"
    );
    o.end().await;
}

/// A record's bytes carry a counter modulo its width (§2.20): a 24-bit one
/// is read by its increase from the last count, so its wrap is written; a
/// 64-bit one is its count, refused below the last. A series field's code
/// its type does not read is refused (§2.18); a reserved code is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_records_bytes_carry_each_counter_by_its_code() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let _r = reader(&rs).await;
    let o = start(&ep, None).await;
    let c = meterframe();
    let bytes_of = |template: &str, v: Value| -> Vec<u8> {
        let Body::Data(d) = &resource(&c, template).body else {
            unreachable!()
        };
        c.packed_layout(&d.type_)
            .unwrap()
            .unwrap()
            .write(&v, Some(0))
            .unwrap()
            .bytes
    };
    let t = Instant::now();
    let w = o.w(BYTES);
    w.record_at(&bytes_of(BYTES, json!({"rx": 16_777_000})), t)
        .await
        .unwrap();
    // 100 is 16,777,316 modulo 2^24: an increase of 316, written.
    w.record_at(&bytes_of(BYTES, json!({"rx": 100})), t + s(1))
        .await
        .unwrap();
    // As a count, 16,777,310 is below it.
    let e = w.record_json_at(&json!({"rx": 16_777_310}), t + s(2)).await;
    assert_eq!(
        refusal(e.unwrap_err()),
        Reason::FieldDecreased {
            field: "rx".into(),
            before: 16_777_316,
            after: 16_777_310,
        }
    );
    let w = o.w(TOTALS);
    let total = |n: u64| bytes_of(TOTALS, json!({"frames_total": n, "temperature": "error"}));
    w.record_at(&total(500), t).await.unwrap();
    let e = w.record_at(&total(480), t + s(1)).await.unwrap_err();
    assert!(matches!(refusal(e), Reason::FieldDecreased { .. }));
    // duty_available is an int from 0 to 1, one bit: code 2 is none of it.
    let e = o
        .w(LINK)
        .record_slots_at(
            vec![Slot::Code(7), Slot::Code(3), Slot::Code(2), Slot::Code(0)],
            t,
        )
        .await
        .unwrap_err();
    assert_eq!(
        refusal(e),
        Reason::FieldCode {
            field: "duty_available".into(),
            code: 2,
        }
    );
    o.end().await;
}

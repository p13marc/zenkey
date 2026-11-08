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

/// #1055: a `config.v1` owner whose device resets, its counters with it,
/// mints a new instance (core §1.5), which restarts its configuration
/// (`config.v1` §2.12, "A re-mint is a restart"): its pending change
/// cancelled, its device returned to its startup values, revision 0. R
/// reads a reset across, in band on each frame series (§2.19) and by
/// presence on the bare counter (v1 §2.9), never a decrease.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_config_v1_owners_device_reset_is_a_new_instance() {
    use zenkey::config_v1::{self, ConfigView};
    use zenkey::model::config::{ConfigChangeEvent, Outcome as Ending, Source};
    use zenkey::model::telemetry::Presence;
    use zenkey::{CallMetadata, Client, Outcome};

    /// A call of `config.v1`'s `resource` at O, as `ops`.
    async fn call(s: &zenoh::Session, resource: &str, group: Option<&str>, body: Value) -> Outcome {
        let c = Client::new(s, config_v1::implementation().shared_contract(), &[SVC])
            .unwrap()
            .with_timeout(Duration::from_secs(2))
            .with_metadata(CallMetadata::new("ops", resource));
        let values: Bindings = group
            .map(|g| [("group".to_owned(), vec![g.to_owned()])].into())
            .unwrap_or_default();
        c.call_value(&SVC.parse().unwrap(), resource, &values, &body)
            .await
            .unwrap()
    }
    fn value(o: Outcome) -> Value {
        match o {
            Outcome::Value(a) => a.value::<Value>().unwrap(),
            other => panic!("a value reply, not {other:?}"),
        }
    }
    let param = |v: &ConfigView, g: &str, p: &str| {
        let x = v.groups.iter().find(|x| x.name == g).unwrap();
        let x = x.parameters.iter().find(|x| x.name == p).unwrap();
        (x.value.clone(), x.source)
    };
    let int = |n: i64| Some(config_v1::Value::Integer(n));

    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;
    // R's presence view: each instance token's put and delete, as it
    // arrived; and what it hears of O's configuration.
    type Log<T> = Arc<Mutex<Vec<T>>>;
    let tokens: Log<(Instant, bool, String)> = Arc::default();
    let configs: Log<(String, Vec<u8>)> = Arc::default();
    let (t1, c1) = (Arc::clone(&tokens), Arc::clone(&configs));
    let _tokens = rs
        .liveliness()
        .declare_subscriber("zk2/lab/meterframe/@zk/instance/*")
        .callback(move |x: Sample| {
            let id = x.key_expr().as_str().rsplit('/').next().unwrap().to_owned();
            let put = x.kind() == zenoh::sample::SampleKind::Put;
            t1.lock().unwrap().push((Instant::now(), put, id));
        })
        .await
        .unwrap();
    let _configs = rs
        .declare_subscriber("zk2/lab/meterframe/config.v1/**")
        .callback(move |x: Sample| {
            let key = x.key_expr().as_str().to_owned();
            c1.lock().unwrap().push((key, common::record(&x)));
        })
        .await
        .unwrap();
    let heard_token = |id: &str, put: bool| {
        tokens
            .lock()
            .unwrap()
            .iter()
            .any(|(_, p, i)| *p == put && i == id)
    };
    let views = || -> Vec<ConfigView> {
        configs
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k.ends_with("/state/view"))
            .map(|(_, b)| serde_json::from_slice(b).unwrap())
            .collect()
    };
    let events = || -> Vec<ConfigChangeEvent> {
        configs
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k.contains("/events/change/"))
            .map(|(_, b)| serde_json::from_slice(b).unwrap())
            .collect()
    };

    // O: meterframe.v1's frames and its bare counter, and config.v1 on the
    // owner example's simulated device.
    let session = client(&ep).await;
    let mut b = ServiceBuilder::new(&session, config(SVC));
    let startup = sim::rf_startup(false, 10, Duration::from_secs(1), false);
    let device = Arc::new(sim::SimDevice::new(&startup));
    let cfg = b
        .config_v1(startup, Arc::clone(&device) as Arc<dyn config_v1::Device>)
        .await
        .unwrap();
    let mut o = owner_of(session, b).await;
    let old = o.svc.instance().to_string();
    eventually("R sees O's instance token", || async {
        heard_token(&old, true)
    })
    .await;

    // 1. A runtime change made permanent (duty, hot, no window), then one
    // pending, applied (radio, reach, a 60 s window).
    let applied = value(
        call(
            &rs,
            config_v1::SET,
            Some("duty"),
            json!({"values": {"fraction_ppm": 20000}}),
        )
        .await,
    );
    assert!(applied.get("applied").is_some(), "{applied}");
    let pending = value(
        call(
            &rs,
            config_v1::SET,
            Some("radio"),
            json!({"values": {"frequency_hz": 869525000}, "confirm_s": 60}),
        )
        .await,
    );
    let token = pending["pending"]["token"].as_str().unwrap().to_owned();
    eventually("the reach change applied", || async {
        cfg.view().await.pending.is_some_and(|p| p.applied)
    })
    .await;
    assert_eq!(device.holds("radio", "frequency_hz"), int(869_525_000));

    // 2. Its counters, in instance I1: 100 then 120, rx 100 left in the
    // open frame, the bare counter 40 then 50.
    let t = Instant::now();
    let r0 = o.totals(100, "21.5", t).await.run;
    o.totals(120, "21.5", t + s(1)).await;
    assert_eq!(o.record(BYTES, json!({"rx": 100}), t).await, None);
    for v in [40u64, 50] {
        o.dropped.put_value(&v).await.unwrap();
    }
    r.wait(DROPPED, 2).await;

    // 3. The device resets, its counters from 0: within I1 each is below
    // its last, refused (v1 §2.7, v2 §2.19).
    let below = json!({"frames_total": 5, "temperature": "21.5"});
    let e = o.w(TOTALS).record_json_at(&below, t + s(2)).await;
    assert!(matches!(
        refusal(e.unwrap_err()),
        Reason::FieldDecreased { .. }
    ));
    let e = o.dropped.put_value(&3u64).await.unwrap_err();
    assert!(matches!(refusal(e), Reason::Decreased { .. }));

    // 4. The open frame flushed under I1's run, then a new instance, I2.
    assert_eq!(o.w(BYTES).flush().await.unwrap().expect("rx 100").run, r0);
    let new = o.svc.new_epoch().await.unwrap().to_string();
    assert_ne!(new, old);
    let r1 = o.svc.minter().run();
    assert_ne!(r1, r0, "a new instance takes a new run (core §4.3)");
    eventually("R sees I1's token withdrawn", || async {
        heard_token(&old, false)
    })
    .await;

    // 5. The restarted counts, in I2.
    let b = o.totals(5, "21.5", t + s(2)).await;
    assert_eq!((b.run, b.seq.get()), (r1, 0));
    o.totals(9, "21.5", t + s(3)).await;
    assert_eq!(o.record(BYTES, json!({"rx": 5}), t + s(2)).await, None);
    assert_eq!(o.w(BYTES).flush().await.unwrap().expect("rx 5").run, r1);
    for v in [3u64, 4] {
        o.dropped.put_value(&v).await.unwrap();
    }
    r.wait(TOTALS, 4).await;
    r.wait(BYTES, 2).await;
    r.wait(DROPPED, 4).await;

    // R, the frames: 120 → 5 and rx 100 → 5 resets in band, with no
    // rate; I2's first frame another run's, a keyframe; no decrease.
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

    // R, the bare counter, by v1's rules with its presence view: 50 → 3 a
    // reset, I1 alone at 50's arrival and I2 at 3's; within each instance,
    // the same epoch.
    let at = |i: Instant| i128::try_from(i.duration_since(r.origin).as_nanos()).unwrap();
    let marks: Vec<(i128, bool, String)> = tokens
        .lock()
        .unwrap()
        .iter()
        .map(|(i, p, id)| (at(*i), *p, id.clone()))
        .collect();
    let present = |ns: i128| {
        let mut held: Vec<&str> = Vec::new();
        for (when, put, id) in &marks {
            if *when <= ns {
                if *put {
                    held.push(id);
                } else {
                    held.retain(|h| h != id);
                }
            }
        }
        held
    };
    let only = |ns: i128| match present(ns)[..] {
        [one] => Some(one.to_owned()),
        _ => None,
    };
    let samples = r.bare();
    let mut pairs = Vec::new();
    for w in samples.windows(2) {
        let ((a, a_at), (b, b_at)) = (&w[0], &w[1]);
        let (earlier, later) = (only(*a_at), only(*b_at));
        let moved = marks
            .iter()
            .any(|(when, ..)| *when > *a_at && *when <= *b_at);
        let presence = Presence {
            alone_throughout: earlier.is_some() && earlier == later && !moved,
            earlier: earlier.as_deref(),
            later: later.as_deref(),
        };
        fn sampled(m: &Measure, at: i128) -> Sampled<'_> {
            Sampled {
                value: m,
                epoch: None,
                timestamp_ms: None,
                stamp_ns: None,
                arrival_ns: at,
                record: None,
            }
        }
        let p = pair(
            &sampled(a, *a_at),
            &sampled(b, *b_at),
            Some("1"),
            Some(&presence),
        );
        pairs.push((counter(a.clone()), counter(b.clone()), p));
    }
    let by_presence = Some(Basis::Established(Via::Presence));
    let verdicts: Vec<(u64, u64, Continuity, Option<Basis>)> = pairs
        .iter()
        .map(|(a, b, p)| (*a, *b, p.continuity, p.basis))
        .collect();
    assert_eq!(
        verdicts,
        [
            (40, 50, Continuity::SameEpoch, by_presence),
            (50, 3, Continuity::Reset, by_presence),
            (3, 4, Continuity::SameEpoch, by_presence),
        ]
    );

    // config.v1 restarted in I2 (§2.12): the pending change cancelled, its
    // undo run and its event published; the device returned to its
    // startup values (duty), then revision 0 from them, nothing pending,
    // no last change.
    let v = cfg.view().await;
    assert_eq!((v.instance.as_str(), v.revision), (new.as_str(), 0));
    assert_eq!((&v.pending, &v.last_change), (&None, &None));
    assert_eq!(
        param(&v, "duty", "fraction_ppm"),
        (int(10_000), Some(Source::File))
    );
    assert_eq!(
        param(&v, "radio", "frequency_hz"),
        (int(868_100_000), Some(Source::File))
    );
    assert_eq!(device.holds("duty", "fraction_ppm"), int(10_000));
    assert_eq!(device.holds("radio", "frequency_hz"), int(868_100_000));
    let (undos, restores) = device.with(|d| (d.undos.len(), d.restores.clone()));
    assert_eq!(undos, 1, "the cancel's undo");
    assert_eq!(
        restores.len(),
        1,
        "one return, of what runs off its startup value"
    );
    assert_eq!(
        restores[0]
            .1
            .iter()
            .map(|x| (x.group.as_str(), x.parameter.as_str(), x.value.clone()))
            .collect::<Vec<_>>(),
        [("duty", "fraction_ppm", config_v1::Value::Integer(10_000))]
    );
    eventually("R hears the cancel's event and I2's view", || async {
        events()
            .iter()
            .any(|e| e.token == token && e.outcome == Ending::RolledBack)
            && views().iter().any(|v| v.instance == new && v.revision == 0)
    })
    .await;
    // A late confirm of I1's token names nothing (§2.6); I2 mints its own.
    match call(&rs, config_v1::CONFIRM, None, json!({"token": token})).await {
        Outcome::Refused(e) => assert_eq!(e.code, "not_found"),
        other => panic!("a refusal, not {other:?}"),
    }
    let applied = value(
        call(
            &rs,
            config_v1::SET,
            Some("duty"),
            json!({"values": {"fraction_ppm": 30000}}),
        )
        .await,
    );
    let v: ConfigView = serde_json::from_value(applied["applied"].clone()).unwrap();
    assert_eq!(v.last_change.map(|l| l.token), Some(format!("{new}-1")));
    o.end().await;
}

/// #1064: a `config.v2` owner whose device resets mints a new instance
/// (core §1.5), which restarts its configuration as `config.v1`'s does
/// (`config.v1` §2.12, which `config.v2` §2.12 keeps): its pending change
/// cancelled, its device returned to its startup values, revision 0, and
/// its record's instance number the new instance's run, put before its
/// token. R reads the frame counter across as a reset, in band, never a
/// decrease (§2.19).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_config_v2_owners_device_reset_is_a_new_instance() {
    use zenkey::config_v2::{self, Interface, State};
    use zenkey::model::config::v2::{ChangeEvent, ParamState};
    use zenkey::model::config::{Outcome as Ending, Source, Value as V};
    use zenkey::{CallMetadata, Client, Outcome};

    /// A call of the config contract's `resource` at O, as `ops`.
    async fn call(s: &zenoh::Session, c: &Arc<Contract>, resource: &str, rec: Vec<u8>) -> Outcome {
        Client::new(s, Arc::clone(c), &[SVC])
            .unwrap()
            .with_timeout(Duration::from_secs(2))
            .with_metadata(CallMetadata::new("ops", resource))
            .call(&SVC.parse().unwrap(), resource, &Bindings::new(), rec)
            .await
            .unwrap()
    }
    let rfcfg = Arc::new(sim::rfcfg().unwrap());
    let i = Interface::of(&rfcfg).unwrap();
    let record = |o: &Outcome| -> State {
        let Outcome::Value(a) = o else {
            panic!("a value reply, not {o:?}")
        };
        let bytes = a.record().expect("the value form's record");
        i.read_record(a.tag().unwrap().get(), &bytes, 0).unwrap()
    };
    let set = |group: &str, fields: Value| {
        i.derived
            .set(group)
            .unwrap()
            .1
            .write(&fields, None)
            .unwrap()
            .bytes
    };
    let control = |token: u64| {
        i.derived
            .control()
            .write(&json!({"token": token}), None)
            .unwrap()
            .bytes
    };
    let param = |s: &State, g: &str, p: &str| match &s.groups[g].params[p] {
        ParamState::Plain { source, value, .. } => (value.clone(), *source),
        other => panic!("{g}.{p}: {other:?}"),
    };
    let int = |n: i64| Some(V::Integer(n));

    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;
    // R's view of O's instance tokens, and of its record and events, in
    // arrival order.
    type Log<T> = Arc<Mutex<Vec<T>>>;
    let heard: Log<(Instant, String, Vec<u8>)> = Arc::default();
    let (h1, h2) = (Arc::clone(&heard), Arc::clone(&heard));
    let _tokens = rs
        .liveliness()
        .declare_subscriber("zk2/lab/meterframe/@zk/instance/*")
        .callback(move |x: Sample| {
            if x.kind() == zenoh::sample::SampleKind::Put {
                let key = x.key_expr().as_str().to_owned();
                h1.lock().unwrap().push((Instant::now(), key, Vec::new()));
            }
        })
        .await
        .unwrap();
    let _configs = rs
        .declare_subscriber("zk2/lab/meterframe/rfcfg.v1/**")
        .callback(move |x: Sample| {
            let key = x.key_expr().as_str().to_owned();
            let bytes = x.payload().to_bytes().into_owned();
            h2.lock().unwrap().push((Instant::now(), key, bytes));
        })
        .await
        .unwrap();
    let records = || -> Vec<(Instant, State)> {
        heard
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, k, _)| k.ends_with("/rfcfg.v1/state/config"))
            .map(|(at, _, b)| {
                let e = Envelope::parse(b).unwrap();
                (*at, i.read_record(e.tag.get(), e.record, 0).unwrap())
            })
            .collect()
    };
    let events = || -> Vec<ChangeEvent> {
        heard
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, k, _)| k.contains("/events/change/"))
            .map(|(_, k, b)| {
                // An event (core §7.5's form 4), its stamp its key's.
                let e = zenkey::model::wire::read_event(k, b).unwrap().unwrap();
                serde_json::from_slice(e.record).unwrap()
            })
            .collect()
    };
    let token_at = |id: &str| {
        heard
            .lock()
            .unwrap()
            .iter()
            .find(|(_, k, _)| k.ends_with(&format!("/{id}")))
            .map(|(at, ..)| *at)
    };

    // O: meterframe.v1's frames, and config.v2 over rfcfg.v1 on the owner
    // example's simulated device.
    let session = client(&ep).await;
    // O's order at its source (§2.3), on O's own session: zenoh hands a
    // session's own puts and token declarations to its local subscribers
    // as each is made, so their order there is O's. R's arrivals through
    // the router can carry a token's declaration ahead of a put made
    // before it, the two on different priorities (#1028).
    let at_source: Log<String> = Arc::default();
    let (l1, l2) = (Arc::clone(&at_source), Arc::clone(&at_source));
    let _local_tokens = session
        .liveliness()
        .declare_subscriber("zk2/lab/meterframe/@zk/instance/*")
        .callback(move |x: Sample| {
            if x.kind() == zenoh::sample::SampleKind::Put {
                l1.lock().unwrap().push(x.key_expr().as_str().to_owned());
            }
        })
        .await
        .unwrap();
    let local_record = i.clone();
    let _local_records = session
        .declare_subscriber("zk2/lab/meterframe/rfcfg.v1/state/config")
        .callback(move |x: Sample| {
            let bytes = x.payload().to_bytes();
            let e = Envelope::parse(&bytes).unwrap();
            let st = local_record.read_record(e.tag.get(), e.record, 0).unwrap();
            l2.lock().unwrap().push(format!("record {}", st.instance));
        })
        .await
        .unwrap();
    let mut b = ServiceBuilder::new(&session, config(SVC));
    b.implement(Implementation::new((*rfcfg).clone())).unwrap();
    let startup = sim::rfcfg_startup(10, Duration::from_secs(1), false);
    let device = Arc::new(sim::SimDevice::new_v2(&startup));
    let cfg = b
        .config_v2(startup, Arc::clone(&device) as Arc<dyn config_v2::Device>)
        .await
        .unwrap();
    let mut o = owner_of(session, b).await;
    let old = o.svc.instance().to_string();
    let i1 = cfg.instance();
    eventually("R sees O's instance token", || async {
        token_at(&old).is_some()
    })
    .await;

    // 1. A runtime change made permanent (duty, hot, no window), then one
    // pending, applied (radio, reach, a 60 s window).
    let applied = call(
        &rs,
        &rfcfg,
        &config_v2::set("duty"),
        set("duty", json!({"fraction_ppm": 20000, "dry_run": false})),
    )
    .await;
    assert_eq!(record(&applied).revision, 1);
    let pending = call(
        &rs,
        &rfcfg,
        &config_v2::set("radio"),
        set(
            "radio",
            json!({"frequency_hz": 869_525_000, "confirm_s": 60, "dry_run": false}),
        ),
    )
    .await;
    let token = record(&pending).pending.expect("pending").token;
    eventually("the reach change applied", || async {
        cfg.state().await.pending.is_some_and(|p| p.applied)
    })
    .await;
    assert_eq!(device.holds("radio", "frequency_hz"), int(869_525_000));

    // 2. Its counter, in instance I1: 100 then 120. The device resets, its
    // counter from 0: within I1 below its last, refused (§2.19).
    let t = Instant::now();
    let r0 = o.totals(100, "21.5", t).await.run;
    o.totals(120, "21.5", t + s(1)).await;
    let below = json!({"frames_total": 5, "temperature": "21.5"});
    let e = o.w(TOTALS).record_json_at(&below, t + s(2)).await;
    assert!(matches!(
        refusal(e.unwrap_err()),
        Reason::FieldDecreased { .. }
    ));

    // 3. A new instance, I2: the configuration restarts in it.
    let new = o.svc.new_epoch().await.unwrap().to_string();
    assert_ne!(new, old);
    let r1 = o.svc.minter().run();
    assert_ne!(r1, r0, "a new instance takes a new run (core §4.3)");
    let b = o.totals(5, "21.5", t + s(2)).await;
    assert_eq!((b.run, b.seq.get()), (r1, 0));
    o.totals(9, "21.5", t + s(3)).await;
    r.wait(TOTALS, 4).await;

    // R: 120 → 5 a reset in band, with no rate; no pair a decrease.
    let f = r.fold(TOTALS);
    let p = f.pair("frames_total", 120, 5);
    assert_eq!((p.continuity, p.basis), (Continuity::Reset, IN_BAND));
    assert_eq!(p.rate, Err(NoRate::Reset));
    assert!(
        f.pairs("frames_total")
            .iter()
            .all(|(_, _, p)| p.continuity != Continuity::Decreased),
        "{:?}",
        f.pairs("frames_total")
    );

    // config.v2 restarted in I2 (§2.12): the pending change cancelled, its
    // undo run and its event published; the device returned to its startup
    // values (duty); then revision 0 from them, nothing pending, no last
    // change, the instance number I2's run.
    let i2 = cfg.instance();
    assert_eq!(i2, r1.value(), "the instance number is the new run (§2.3)");
    assert_ne!(i2, i1);
    let st = cfg.state().await;
    assert_eq!((st.instance, st.revision), (i2, 0));
    assert_eq!((&st.pending, &st.last), (&None, &None));
    assert_eq!(
        param(&st, "duty", "fraction_ppm"),
        (int(10_000), Some(Source::File))
    );
    assert_eq!(
        param(&st, "radio", "frequency_hz"),
        (int(868_100_000), Some(Source::File))
    );
    assert_eq!(device.holds("duty", "fraction_ppm"), int(10_000));
    assert_eq!(device.holds("radio", "frequency_hz"), int(868_100_000));
    let (undos, restores) = device.with(|d| (d.undos.len(), d.restores.clone()));
    assert_eq!(undos, 1, "the cancel's undo");
    assert_eq!(
        restores
            .iter()
            .flat_map(|(_, r)| r.iter())
            .map(|x| (x.group.as_str(), x.parameter.as_str(), x.value.clone()))
            .collect::<Vec<_>>(),
        [("duty", "fraction_ppm", V::Integer(10_000))],
        "one return, of what runs off its startup value"
    );
    eventually("R hears the cancel's event and I2's record", || async {
        events()
            .iter()
            .any(|e| e.token == token && e.outcome == Ending::RolledBack)
            && records()
                .iter()
                .any(|(_, s)| s.instance == i2 && s.revision == 0)
    })
    .await;
    eventually("R sees I2's token", || async { token_at(&new).is_some() }).await;
    let order = at_source.lock().unwrap().clone();
    let record_at = order
        .iter()
        .position(|e| *e == format!("record {i2}"))
        .unwrap_or_else(|| panic!("I2's record at its source: {order:?}"));
    let token_at_source = order
        .iter()
        .position(|e| e.ends_with(&format!("/{new}")))
        .unwrap_or_else(|| panic!("I2's token at its source: {order:?}"));
    assert!(
        record_at < token_at_source,
        "I2's record before its token (§2.3): {order:?}"
    );

    // A late confirm of I1's token names nothing (§2.6); I2 mints its own,
    // under its run.
    let late = call(&rs, &rfcfg, config_v2::CONFIRM, control(token)).await;
    match late {
        Outcome::Refused(e) => assert_eq!(e.code, "not_found"),
        other => panic!("a refusal, not {other:?}"),
    }
    let again = call(
        &rs,
        &rfcfg,
        &config_v2::set("duty"),
        set("duty", json!({"fraction_ppm": 30000, "dry_run": false})),
    )
    .await;
    let last = record(&again).last.expect("the last change").token;
    assert_eq!(last >> 16, u64::from(i2), "a token of I2's run (§2.6)");

    // 4. config.v1's refusals: a device that cannot return to its startup
    // values refuses the re-mint, and I2 runs on.
    device.with(|d| d.refuse_restore = Some("the radio is busy".to_owned()));
    let e = o.svc.new_epoch().await.expect_err("the device refused");
    assert!(e.to_string().contains("config.v1 §2.12"), "{e}");
    assert_eq!(o.svc.instance().to_string(), new);
    assert_eq!(cfg.instance(), i2);
    assert_eq!(
        param(&cfg.state().await, "duty", "fraction_ppm"),
        (int(30_000), Some(Source::Runtime))
    );
    o.end().await;
}

/// A new run within the instance (core §4.3's MAY, [`Service::new_run`]):
/// no counter restarted, no instance minted. The descriptor states the
/// run, and R reads the pair across as a reset, in band, with no rate
/// (§2.19, "A new run within an instance").
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_run_within_the_instance_reads_as_a_reset() {
    let (_r1, ep) = router(None).await;
    let rs = client(&ep).await;
    let r = reader(&rs).await;
    let o = start(&ep, None).await;
    let instance = o.svc.instance().clone();
    let t = Instant::now();
    let r0 = o.totals(100, "21.5", t).await.run;
    let r1 = o.svc.new_run();
    assert_ne!(r1, r0);
    let d: Value = serde_json::from_slice(&o.svc.descriptor_bytes()).unwrap();
    assert_eq!(d["run"], r1.hex(), "the descriptor states it (core §3.3)");
    assert_eq!(o.svc.instance(), &instance, "no new instance");
    let b = o.totals(101, "21.5", t + s(1)).await;
    assert_eq!((b.run, b.seq.get()), (r1, 0));
    r.wait(TOTALS, 2).await;
    let p = r.fold(TOTALS).pair("frames_total", 100, 101);
    assert_eq!((p.continuity, p.basis), (Continuity::Reset, IN_BAND));
    assert_eq!(p.rate, Err(NoRate::Reset));
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

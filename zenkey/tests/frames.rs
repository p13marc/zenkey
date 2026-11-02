//! `spec/scenarios/frames.md` (core §4.3, §7.7; #981's K3): an owner writes
//! a frame resource's records through its `FrameWriter`, and a tool's
//! consumer reads them, one `zenkey_model::frame::Reader` per key.
//!
//! Setup: R1 a router; O the owner at `lab/modem` of `linkframes.v1`
//! (`tests/contracts/frames/linkframes.v1.toml`), a client of R1; S a
//! tool's consumer of it on another client. A frame "lost on its way" is
//! the writer's `simulate_loss`: its seqs taken, nothing put.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::{client, config, contract, eventually, router};
use serde_json::{Value, json};
use zenkey::ServiceBuilder;
use zenkey::consumer::{Consumer, FrameRead, Subscription};
use zenkey::frame::FrameWriter;
use zenkey::implementation::Implementation;
use zenkey::model::frame::{At, FieldState, Follows, Frame, Reader, Slot, Unknown, When};
use zenkey::model::grammar::IfaceId;
use zenkey::model::template::Bindings;
use zenkey::model::wire::{Run, Seq, Stamp};

const LINK: &str = "stream/frames/link";
const SAMPLE: &str = "stream/frames/sample";

fn iface() -> IfaceId {
    "linkframes.v1".parse().unwrap()
}

/// The base record: `dropped` 7, `queue_depth` 12, `duty_available` 1,
/// `duty_wait_ms` "1500".
fn base() -> Value {
    json!({"dropped": 7, "queue_depth": 12, "duty_available": 1, "duty_wait_ms": "1500"})
}

/// The base record with `changes` over it.
fn with(changes: Value) -> Value {
    let mut v = base();
    for (k, x) in changes.as_object().unwrap() {
        v[k] = x.clone();
    }
    v
}

fn sample(t: &str, rssi: i64, mode: &str) -> Value {
    json!({"temperature": t, "rssi": rssi, "link_up": true, "mode": mode})
}

/// One frame as S heard it: what its reader read, and its body.
#[derive(Clone)]
struct Heard {
    read: FrameRead,
    stamp: Stamp,
    body: Vec<u8>,
    tag: zenkey::model::wire::Tag,
    provider: String,
}

impl Heard {
    /// The body, read alone.
    fn frame(&self) -> Frame {
        Frame::parse(self.tag, &self.body, &self.read.layout).unwrap()
    }

    fn state(&self, record: usize, field: &str) -> FieldState {
        self.read.field(record, field).unwrap()
    }
}

type Log = Arc<Mutex<Vec<Heard>>>;

/// R1, O and S's session; O's service started with a frame writer on each
/// frame resource.
struct World {
    _r1: zenoh::Session,
    ep: String,
    svc: zenkey::Service,
    link: FrameWriter,
    sample: FrameWriter,
    rs: zenoh::Session,
}

impl World {
    async fn new() -> World {
        let (r1, ep) = router(None).await;
        let os = client(&ep).await;
        let rs = client(&ep).await;
        let mut b = ServiceBuilder::new(&os, config("lab/modem"));
        b.implement(Implementation::new(contract("frames/linkframes.v1")))
            .unwrap();
        b.expose(&iface(), "stream/level").unwrap();
        let link = b.declare_frame_writer(&iface(), LINK).await.unwrap();
        let sample = b.declare_frame_writer(&iface(), SAMPLE).await.unwrap();
        let svc = b.start().await.unwrap();
        World {
            _r1: r1,
            ep,
            svc,
            link,
            sample,
            rs,
        }
    }

    /// S's subscription to `resource` of `providers`, logging each frame.
    async fn subscribe(&self, resource: &str, providers: &[&str]) -> (Subscription, Log) {
        let log: Log = Arc::default();
        let l = Arc::clone(&log);
        let sub = Consumer::for_tool(
            &self.rs,
            Arc::new(contract("frames/linkframes.v1")),
            providers,
            &Default::default(),
        )
        .unwrap()
        .subscribe(resource, move |d| {
            let (Some(read), Some(stamp), Some(tag)) = (d.frame, d.inband, d.tag) else {
                return;
            };
            l.lock().unwrap().push(Heard {
                read,
                stamp,
                body: d.sample.payload().to_bytes().into_owned(),
                tag,
                provider: d.provider.to_string(),
            });
        })
        .await
        .unwrap();
        (sub, log)
    }
}

/// Waits for O's writer `w` to know S's subscriber: a put is dropped at
/// its source until then.
async fn matched(w: &FrameWriter) {
    eventually("S's subscriber reaches O", || async {
        w.writer().matching().await.unwrap()
    })
    .await;
}

async fn heard(log: &Log, n: usize) -> Vec<Heard> {
    eventually(&format!("{n} frames"), || async {
        log.lock().unwrap().len() >= n
    })
    .await;
    log.lock().unwrap().clone()
}

fn seqs(h: &[Heard]) -> Vec<u32> {
    h.iter().map(|x| x.stamp.seq.get()).collect()
}

fn all_known(h: &Heard, record: usize) -> bool {
    h.read.applied.records[record]
        .fields
        .iter()
        .all(|s| matches!(s, FieldState::Known(_)))
}

/// §1: a keyframe, a delta record, a full record `keyframe_every` records
/// on, a three-record frame, and no mutation; each frame read alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_each_frame_decodes_alone() {
    let w = World::new().await;
    let (_l, link) = w.subscribe(LINK, &["lab/modem"]).await;
    let (_s, samples) = w.subscribe(SAMPLE, &["lab/modem"]).await;
    matched(&w.link).await;
    matched(&w.sample).await;

    // Step 1: a keyframe, then a delta record.
    let s0 = w.link.record_json(&base()).await.unwrap().expect("a frame");
    assert_eq!(s0.seq.get(), 0);
    let s1 = w
        .link
        .record_json(&with(json!({"queue_depth": 30, "duty_wait_ms": "2500"})))
        .await
        .unwrap()
        .expect("a frame");
    assert_eq!((s1.run, s1.seq.get()), (s0.run, 1));
    let h = heard(&link, 2).await;
    assert!(h[0].frame().records[0].full);
    let delta = &h[1].frame().records[0];
    assert!(!delta.full);
    assert_eq!(
        delta.slots,
        [Slot::Omitted, Slot::Code(30), Slot::Omitted, Slot::Code(25)]
    );
    assert_eq!(h[1].read.applied.follows, Follows::Continues);
    assert!(all_known(&h[1], 0));
    assert_eq!(h[1].state(0, "queue_depth"), FieldState::Known(30));
    assert_eq!(h[1].state(0, "dropped"), FieldState::Known(7));

    // Step 2: seqs 2 to 15, a full record 15 records after the last.
    for i in 2..=15u64 {
        w.link
            .record_json(&with(
                json!({"queue_depth": 30 + 10 * (i - 1), "duty_wait_ms": "2500"}),
            ))
            .await
            .unwrap();
    }
    let h = heard(&link, 16).await;
    assert_eq!(seqs(&h), (0..16).collect::<Vec<_>>());
    let full: Vec<bool> = h.iter().map(|x| x.frame().records[0].full).collect();
    assert!(
        full[0] && full[15] && full[1..15].iter().all(|f| !f),
        "{full:?}"
    );

    // Step 3: three records 10 s apart, on their schedule: one frame.
    let t0 = Instant::now();
    let s = &w.sample;
    assert_eq!(
        s.record_json_at(&sample("21.5", -80, "rx"), t0)
            .await
            .unwrap(),
        None
    );
    let ten = Duration::from_secs(10);
    assert_eq!(
        s.record_json_at(&sample("21.5", -82, "rx"), t0 + ten)
            .await
            .unwrap(),
        None
    );
    let put = s
        .record_json_at(&sample("22", -82, "tx"), t0 + 2 * ten)
        .await
        .unwrap()
        .expect("three records");
    assert_eq!(put.seq.get(), 0);
    let h = heard(&samples, 1).await;
    let f = h[0].frame();
    let shape: Vec<(bool, Option<At>)> = f.records.iter().map(|r| (r.full, r.at)).collect();
    assert_eq!(
        shape,
        [
            (true, None),
            (false, Some(At::Scheduled)),
            (false, Some(At::Scheduled))
        ]
    );
    // rssi alone, then temperature and mode.
    assert_eq!(
        f.records[1].slots,
        [Slot::Omitted, Slot::Code(58), Slot::Omitted, Slot::Omitted]
    );
    assert_eq!(
        f.records[2].slots,
        [Slot::Code(124), Slot::Omitted, Slot::Omitted, Slot::Code(2)]
    );
    assert_eq!(
        h[0].read
            .applied
            .records
            .iter()
            .map(|r| r.seq.get())
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    s.record_json_at(&sample("22", -82, "tx"), t0 + 3 * ten)
        .await
        .unwrap();
    assert_eq!(s.flush().await.unwrap().map(|x| x.seq.get()), Some(3));

    // Step 4: no plain writer, so no mutation.
    let refused = w
        .svc
        .writer(&iface(), LINK, &Bindings::new())
        .await
        .err()
        .expect("refused");
    assert!(refused.to_string().contains("frame resource"), "{refused}");

    // Step 5: each frame alone, by a reader holding nothing.
    let alone = |h: &Heard| Reader::new().apply(&h.read.layout, h.stamp, &h.frame());
    let link = link.lock().unwrap().clone();
    let first = alone(&link[0]);
    assert!(
        first.records[0]
            .fields
            .iter()
            .all(|s| matches!(s, FieldState::Known(_)))
    );
    let second = alone(&link[1]).records[0].fields.clone();
    assert_eq!(
        second,
        [
            FieldState::Unknown(Unknown::First),
            FieldState::Known(30),
            FieldState::Unknown(Unknown::First),
            FieldState::Known(25)
        ]
    );
    let three = alone(&heard(&samples, 1).await[0]);
    for r in &three.records {
        assert!(r.fields.iter().all(|s| matches!(s, FieldState::Known(_))));
    }
}

/// §2: a lost delta frame turns the fields its successors omit unknown,
/// never their last values; the next full record heals them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_a_lost_delta_frame() {
    let w = World::new().await;
    let (_l, link) = w.subscribe(LINK, &["lab/modem"]).await;
    matched(&w.link).await;
    w.link.record_json(&base()).await.unwrap();
    w.link
        .record_json(&with(json!({"queue_depth": 30})))
        .await
        .unwrap();
    // Step 2: lost on its way.
    w.link.simulate_loss();
    let lost = w
        .link
        .record_json(&with(json!({"queue_depth": 30, "duty_wait_ms": "2500"})))
        .await
        .unwrap();
    assert_eq!(lost.map(|s| s.seq.get()), Some(2));
    // Step 3.
    w.link
        .record_json(&with(json!({"queue_depth": 40, "duty_wait_ms": "2500"})))
        .await
        .unwrap();
    let h = heard(&link, 3).await;
    assert_eq!(seqs(&h), [0, 1, 3]);
    assert!(all_known(&h[1], 0));
    let g = &h[2];
    assert_eq!(g.read.applied.follows, Follows::Gap { lost: 1 });
    assert_eq!(g.state(0, "queue_depth"), FieldState::Known(40));
    for f in ["dropped", "duty_available", "duty_wait_ms"] {
        let s = g.state(0, f);
        assert_eq!(s, FieldState::Unknown(Unknown::Gap), "{f}");
        assert_eq!(s.code(), None, "{f}: no value, neither the last nor 0");
    }
    let reading = g.read.reading().unwrap();
    assert_eq!(
        reading["records"][0]["fields"]["duty_wait_ms"],
        json!({"state": "unknown", "why": "gap"})
    );
    // Step 4: unknown until seq 15, the next full record.
    for i in 1..=12u64 {
        w.link
            .record_json(&with(
                json!({"queue_depth": 40 + 10 * i, "duty_wait_ms": "2500"}),
            ))
            .await
            .unwrap();
    }
    let h = heard(&link, 15).await;
    assert_eq!(seqs(&h).last(), Some(&15));
    for x in &h[2..14] {
        assert_eq!(
            x.state(0, "duty_wait_ms"),
            FieldState::Unknown(Unknown::Gap),
            "seq {}",
            x.stamp.seq.get()
        );
    }
    assert!(h[14].frame().records[0].full);
    assert!(all_known(&h[14], 0));
    assert_eq!(h[14].state(0, "duty_wait_ms"), FieldState::Known(25));
}

/// §3: a record within its jitter is scheduled, a later one measured; a
/// record past the span ends the frame and opens the next.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_scheduled_and_measured_time() {
    let w = World::new().await;
    let (_s, samples) = w.subscribe(SAMPLE, &["lab/modem"]).await;
    matched(&w.sample).await;
    let s = &w.sample;
    let ms = Duration::from_millis;
    let wall = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let t = Instant::now();
    let v = sample("21.5", -80, "rx");
    s.record_json_at(&v, t).await.unwrap();
    s.record_json_at(&v, t + ms(10_200)).await.unwrap();
    let put = s.record_json_at(&v, t + ms(20_700)).await.unwrap();
    assert_eq!(put.map(|x| x.seq.get()), Some(0));
    let h = heard(&samples, 1).await;
    let r = &h[0].read.applied.records;
    assert_eq!(r[0].when, When::Measured);
    assert_eq!(r[1].when, When::Scheduled { jitter_ms: 500 });
    assert_eq!(r[1].after_ms, 10_000, "its instant, never 10.2 s");
    assert_eq!((r[2].when, r[2].after_ms), (When::Measured, 20_700));
    // Record 0's time is O's wall clock at t, from the deployment epoch 0.
    let at0 = i128::from(r[0].at_ms.expect("a time"));
    let wall = i128::try_from(wall.as_millis()).unwrap();
    assert!((at0 - wall).abs() < 1_000, "{at0} against {wall}");
    assert_eq!(r[1].at_ms, Some(r[0].at_ms.unwrap() + 10_000));
    let reading = h[0].read.reading().unwrap();
    assert_eq!(reading["records"][1]["time"], "scheduled");
    assert_eq!(reading["records"][1]["jitter_ms"], 500);
    assert_eq!(reading["records"][2]["time"], "measured");

    // Step 2: T, T + 9.6 s, then T + 35 s, past the span.
    let big_t = t + ms(30_100);
    s.record_json_at(&v, big_t).await.unwrap();
    s.record_json_at(&v, big_t + ms(9_600)).await.unwrap();
    let put = s.record_json_at(&v, big_t + ms(35_000)).await.unwrap();
    assert_eq!(put.map(|x| x.seq.get()), Some(3));
    let h = heard(&samples, 2).await;
    let r = &h[1].read.applied.records;
    assert_eq!(r.len(), 2);
    assert_eq!((r[0].seq.get(), r[1].seq.get()), (3, 4));
    assert_eq!(r[1].when, When::Scheduled { jitter_ms: 500 });
    assert_eq!(s.open().await, 1);
    // Step 3: the open frame, its one record at seq 5.
    assert_eq!(s.flush().await.unwrap().map(|x| x.seq.get()), Some(5));
    let h = heard(&samples, 3).await;
    assert_eq!(h[2].read.applied.records.len(), 1);
    assert_eq!(h[2].read.applied.follows, Follows::Continues);
}

/// §4: a deadband field omitted while within its delta reads `within`, is
/// included every `max_records`, and is `unknown` past it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_the_deadband() {
    let w = World::new().await;
    let (_l, link) = w.subscribe(LINK, &["lab/modem", "lab/rogue"]).await;
    matched(&w.link).await;
    w.link.record_json(&base()).await.unwrap();
    for q in [14, 15, 9, 16] {
        w.link
            .record_json(&with(json!({"queue_depth": q})))
            .await
            .unwrap();
    }
    w.link
        .record_json(&with(json!({"queue_depth": 13})))
        .await
        .unwrap();
    w.link
        .record_json(&with(json!({"queue_depth": 20})))
        .await
        .unwrap();
    let h = heard(&link, 7).await;
    assert_eq!(h[0].state(0, "queue_depth"), FieldState::Known(12));
    for x in &h[1..5] {
        assert_eq!(x.frame().records[0].slots[1], Slot::Omitted);
        assert_eq!(x.state(0, "queue_depth"), FieldState::Within(12));
        let reading = x.read.reading().unwrap();
        assert_eq!(
            reading["records"][0]["fields"]["queue_depth"],
            json!({"state": "within", "value": 12, "delta": "4"})
        );
    }
    assert_eq!(h[5].state(0, "queue_depth"), FieldState::Known(13));
    assert_eq!(h[6].state(0, "queue_depth"), FieldState::Known(20));

    // Step 5: X omits the queue past `max_records`.
    let xs = client(&w.ep).await;
    let key = "zk2/lab/rogue/linkframes.v1/stream/frames/link";
    let p = xs.declare_publisher(key).await.unwrap();
    eventually("S's subscriber reaches X", || async {
        p.matching_status().await.unwrap().matching()
    })
    .await;
    let l = w.link.layout();
    let stamp = |seq: u32| Stamp {
        run: Run::U16(9),
        seq: Seq::new(seq).unwrap(),
    };
    let key0 = json!({"time": {"s": 100, "ms": 0}, "records": [{"full": true, "values": base()}]});
    let put = |v: &Value, seq: u32| {
        Frame::from_json(v, l, None)
            .unwrap()
            .envelope(l, stamp(seq))
            .unwrap()
    };
    p.put(put(&key0, 0)).await.unwrap();
    for seq in 1..=5 {
        let d = json!({"time": {"s": 100 + seq, "ms": 0}, "records": [{"full": false}]});
        p.put(put(&d, seq)).await.unwrap();
    }
    eventually("X's six frames", || async {
        link.lock()
            .unwrap()
            .iter()
            .filter(|x| x.provider == "lab/rogue")
            .count()
            == 6
    })
    .await;
    let x: Vec<Heard> = link
        .lock()
        .unwrap()
        .iter()
        .filter(|x| x.provider == "lab/rogue")
        .cloned()
        .collect();
    for y in &x[1..5] {
        assert_eq!(y.state(0, "queue_depth"), FieldState::Within(12));
    }
    assert_eq!(
        x[5].state(0, "queue_depth"),
        FieldState::Unknown(Unknown::Deadband)
    );
}

/// §5: a new run's first frame is full, and a delta under a run the
/// reader does not hold reads every omitted field unknown.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_new_run() {
    let mut w = World::new().await;
    let (_l, link) = w.subscribe(LINK, &["lab/modem"]).await;
    matched(&w.link).await;
    let a = w.link.record_json(&base()).await.unwrap().unwrap();
    w.link
        .record_json(&with(json!({"queue_depth": 30})))
        .await
        .unwrap();
    // Step 2: a new run, the same record.
    w.svc.new_epoch().await.unwrap();
    let b = w
        .link
        .record_json(&with(json!({"queue_depth": 30})))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(b.run, a.run);
    assert_eq!(b.seq.get(), 0);
    let h = heard(&link, 3).await;
    assert!(
        h[2].frame().records[0].full,
        "a keyframe, though nothing changed"
    );
    assert_eq!(h[2].read.applied.follows, Follows::Run);
    assert!(all_known(&h[2], 0));
    // Step 3: another run; its keyframe lost, then a delta.
    w.svc.new_epoch().await.unwrap();
    w.link.simulate_loss();
    w.link
        .record_json(&with(json!({"queue_depth": 40})))
        .await
        .unwrap();
    let c = w
        .link
        .record_json(&with(json!({"queue_depth": 50})))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(c.seq.get(), 1);
    assert_ne!(c.run, b.run);
    let h = heard(&link, 4).await;
    let d = &h[3];
    assert_eq!(d.stamp, c);
    assert!(!d.frame().records[0].full);
    assert_eq!(d.read.applied.follows, Follows::Run);
    assert_eq!(d.state(0, "queue_depth"), FieldState::Known(50));
    for f in ["dropped", "duty_available", "duty_wait_ms"] {
        assert_eq!(d.state(0, f), FieldState::Unknown(Unknown::Run), "{f}");
    }
}

/// §6: a late joiner reads its first frame's omitted fields unknown, never
/// 0, until the next full record.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_a_late_joiner() {
    let w = World::new().await;
    for i in 0..5u64 {
        w.link
            .record_json(&with(json!({"queue_depth": 12 + 10 * i})))
            .await
            .unwrap();
    }
    let (_l, link) = w.subscribe(LINK, &["lab/modem"]).await;
    matched(&w.link).await;
    for i in 5..16u64 {
        w.link
            .record_json(&with(json!({"queue_depth": 12 + 10 * i})))
            .await
            .unwrap();
    }
    let h = heard(&link, 11).await;
    assert_eq!(seqs(&h), (5..16).collect::<Vec<_>>());
    assert_eq!(h[0].read.applied.follows, Follows::First);
    assert_eq!(h[0].state(0, "queue_depth"), FieldState::Known(62));
    for x in &h[..10] {
        for f in ["dropped", "duty_available", "duty_wait_ms"] {
            let s = x.state(0, f);
            assert_eq!(s, FieldState::Unknown(Unknown::First), "{f}");
            assert_eq!(s.code(), None, "{f}: never 0");
        }
    }
    assert!(h[10].frame().records[0].full);
    assert!(all_known(&h[10], 0));
}

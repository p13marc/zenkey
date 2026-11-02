//! A consumer reads frames (core §7.7, "The reader's fields"; #981's K2):
//! no owner writes a frame yet (K3), so these tests put hand-written ones,
//! raw and enveloped, on a frame resource's key, and read them through a
//! tool's consumer, one `zenkey_model::frame::Reader` per key.
//!
//! Setup: R1 a router; O a raw client putting `lab/modem`'s frames of
//! `linkframes.v1` (`tests/contracts/linkframes.v1.toml`); S a tool's
//! consumer of it on another client.

mod common;

use std::sync::{Arc, Mutex};

use common::{client, client_ns, contract, eventually, router};
use serde_json::{Value, json};
use zenkey::consumer::{Consumer, Delivery, FrameRead, Refused};
use zenkey::model::frame::{FieldState, Follows, Frame, Layout, Unknown, When};
use zenkey::model::wire::{Envelope, Run, Seq, Stamp};

const LINK: &str = "zk2/lab/modem/linkframes.v1/stream/frames/link";
const SAMPLE: &str = "zk2/lab/modem/linkframes.v1/stream/frames/sample";
const LEVEL: &str = "zk2/lab/modem/linkframes.v1/stream/level";

fn layout(template: &str) -> Layout {
    let c = contract("linkframes.v1");
    let r = c
        .resources
        .iter()
        .find(|r| r.template.as_str() == template)
        .unwrap();
    Layout::of_resource(&c, r)
        .unwrap()
        .expect("a frame resource")
}

fn stamp(seq: u32) -> Stamp {
    Stamp {
        run: Run::U16(7),
        seq: Seq::new(seq).unwrap(),
    }
}

/// A frame's put: `value` in `spec/conformance/frame/bodies.json`'s
/// spelling, under `l`'s tag, its record 0 at `seq`.
fn put(l: &Layout, seq: u32, value: &Value) -> Vec<u8> {
    Frame::from_json(value, l, None)
        .unwrap()
        .envelope(l, stamp(seq))
        .unwrap()
}

fn keyframe(t: u32, queue: u64) -> Value {
    json!({"time": {"s": t, "ms": 0}, "records": [{"full": true, "values": {
        "dropped": 7, "queue_depth": queue, "duty_available": 1, "duty_wait_ms": "1500"}}]})
}

fn delta(t: u32, values: Value) -> Value {
    json!({"time": {"s": t, "ms": 0}, "records": [{"full": false, "values": values}]})
}

/// Each delivery's frame, and each refusal's token, in order.
#[derive(Default)]
struct Heard {
    frames: Mutex<Vec<FrameRead>>,
    refused: Mutex<Vec<&'static str>>,
}

/// S's subscription to `resource`, hearing into `heard`.
async fn subscribe(
    rs: &zenoh::Session,
    resource: &str,
    heard: &Arc<Heard>,
) -> zenkey::consumer::Subscription {
    let (h, r) = (Arc::clone(heard), Arc::clone(heard));
    Consumer::for_tool(
        rs,
        Arc::new(contract("linkframes.v1")),
        &["lab/modem"],
        &Default::default(),
    )
    .unwrap()
    .with_refused(move |x: Refused| r.refused.lock().unwrap().push(x.why.token()))
    .subscribe(resource, move |d: Delivery| {
        h.frames.lock().unwrap().extend(d.frame);
    })
    .await
    .unwrap()
}

/// A keyframe, a delta, a lost delta, a deadband, a refused body, a
/// mutation, and a keyframe that heals: the consumer delivers each frame
/// with its fields' states, and refuses what has no reading.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_subscription_reads_a_keys_frames_into_field_states() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let heard = Arc::new(Heard::default());
    let sub = subscribe(&rs, "stream/frames/link", &heard).await;
    let p = os.declare_publisher(LINK).await.unwrap();
    eventually("S's subscriber reaches O", || async {
        p.matching_status().await.unwrap().matching()
    })
    .await;
    let l = layout("frames/link");
    let count = |n: usize| {
        let h = Arc::clone(&heard);
        move || {
            let h = Arc::clone(&h);
            async move { h.frames.lock().unwrap().len() + h.refused.lock().unwrap().len() >= n }
        }
    };
    let states = |i: usize| -> Vec<FieldState> {
        heard.frames.lock().unwrap()[i].applied.records[0]
            .fields
            .clone()
    };

    // 10: a keyframe, read first; 11: the queue changed.
    p.put(put(&l, 10, &keyframe(100, 12))).await.unwrap();
    p.put(put(&l, 11, &delta(160, json!({"queue_depth": 30}))))
        .await
        .unwrap();
    eventually("two frames", count(2)).await;
    {
        let f = heard.frames.lock().unwrap();
        assert_eq!(f[0].applied.follows, Follows::First);
        assert_eq!(f[1].applied.follows, Follows::Continues);
        assert_eq!(f[1].field(0, "queue_depth"), Some(FieldState::Known(30)));
        assert_eq!(f[1].field(0, "dropped"), Some(FieldState::Known(7)));
        assert_eq!(f[1].applied.records[0].when, When::Measured);
        assert_eq!(f[1].applied.records[0].at_ms, Some(160_000));
    }

    // 12 is lost; 13 includes the wait alone: the rest unknown, never the
    // last value.
    p.put(put(&l, 13, &delta(280, json!({"duty_wait_ms": "2500"}))))
        .await
        .unwrap();
    eventually("a frame after a gap", count(3)).await;
    assert_eq!(
        heard.frames.lock().unwrap()[2].applied.follows,
        Follows::Gap { lost: 1 }
    );
    let s = states(2);
    assert_eq!(s[0], FieldState::Unknown(Unknown::Gap));
    assert_eq!(s[1], FieldState::Unknown(Unknown::Gap));
    assert_eq!(s[1].code(), None);
    assert!(matches!(s[3], FieldState::Known(_)));

    // 14 includes the queue, 15 omits it: within its delta of 4.
    p.put(put(&l, 14, &delta(340, json!({"queue_depth": 31}))))
        .await
        .unwrap();
    p.put(put(&l, 15, &delta(400, json!({})))).await.unwrap();
    eventually("the deadband's frames", count(5)).await;
    assert_eq!(states(4)[1], FieldState::Within(31));
    let reading = heard.frames.lock().unwrap()[4].reading().unwrap();
    assert_eq!(
        reading["records"][0]["fields"]["queue_depth"],
        json!({"state": "within", "value": 31, "delta": "4"})
    );
    assert_eq!(
        reading["records"][0]["fields"]["dropped"],
        json!({"state": "unknown", "why": "gap"})
    );

    // Another tag's frame, and a mutation on the frame key: refused, each
    // named; their seqs are lost to the reader.
    let mut other = put(&l, 16, &keyframe(460, 12));
    other[0] = (other[0] & 0xf0) | ((other[0] + 1) & 0x0f);
    p.put(other).await.unwrap();
    p.put(Envelope::mutation(l.tag(), stamp(17), &[0; 8]).write())
        .await
        .unwrap();
    eventually("two refusals", count(7)).await;
    assert_eq!(*heard.refused.lock().unwrap(), ["tag", "mutation"]);
    assert_eq!(sub.undecodable(), 2);

    // 18, a keyframe: two records lost, and every field healed.
    p.put(put(&l, 18, &keyframe(580, 40))).await.unwrap();
    eventually("the healing keyframe", count(8)).await;
    assert_eq!(
        heard.frames.lock().unwrap()[5].applied.follows,
        Follows::Gap { lost: 2 }
    );
    // The gap counts the refused frame's seq, and names it; the mutation
    // was no frame, so only its seq is lost.
    assert_eq!(
        heard.frames.lock().unwrap()[5].applied.refused,
        [Seq::new(16).unwrap()]
    );
    assert!(states(5).iter().all(|s| matches!(s, FieldState::Known(_))));
}

/// In a namespace, the consumer's session hears each key base-relative,
/// and its reader of the key keeps the run across frames: the second frame
/// continues the first.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_namespaced_subscription_keeps_one_reader_per_key() {
    let (_r1, ep) = router(None).await;
    let os = client_ns(&ep, Some("site")).await;
    let rs = client_ns(&ep, Some("site")).await;
    let heard = Arc::new(Heard::default());
    let _sub = subscribe(&rs, "stream/frames/link", &heard).await;
    let p = os.declare_publisher(LINK).await.unwrap();
    eventually("S's subscriber reaches O", || async {
        p.matching_status().await.unwrap().matching()
    })
    .await;
    let l = layout("frames/link");
    p.put(put(&l, 1, &keyframe(100, 12))).await.unwrap();
    p.put(put(&l, 2, &delta(160, json!({})))).await.unwrap();
    eventually("two frames", || async {
        heard.frames.lock().unwrap().len() >= 2
    })
    .await;
    let f = heard.frames.lock().unwrap();
    assert_eq!(f[0].applied.follows, Follows::First);
    assert_eq!(f[1].applied.follows, Follows::Continues);
    assert_eq!(f[1].field(0, "queue_depth"), Some(FieldState::Within(12)));
}

/// A three-record frame's later records are scheduled within its jitter
/// or measured at their offset, each at its own seq; a frame on a plain
/// stream of the same record type has no reading.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_frames_records_keep_scheduled_and_measured_time_apart() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let heard = Arc::new(Heard::default());
    let _sub = subscribe(&rs, "stream/frames/sample", &heard).await;
    let level = Arc::new(Heard::default());
    let _lsub = subscribe(&rs, "stream/level", &level).await;
    let p = os.declare_publisher(SAMPLE).await.unwrap();
    let q = os.declare_publisher(LEVEL).await.unwrap();
    eventually("S's subscribers reach O", || async {
        p.matching_status().await.unwrap().matching()
            && q.matching_status().await.unwrap().matching()
    })
    .await;
    let l = layout("frames/sample");
    let s0 = json!({"temperature": "21.5", "rssi": -85, "link_up": true, "mode": "rx"});
    let v = json!({"time": {"s": 7200, "ms": 0}, "records": [
        {"full": true, "values": s0},
        {"full": false, "at": "scheduled", "values": {"temperature": "22"}},
        {"full": false, "at": 20750, "values": {"rssi": -90}}]});
    p.put(put(&l, 30, &v)).await.unwrap();
    q.put(put(&l, 0, &v)).await.unwrap();
    eventually("the frame and the refusal", || async {
        !heard.frames.lock().unwrap().is_empty() && !level.refused.lock().unwrap().is_empty()
    })
    .await;
    let f = heard.frames.lock().unwrap()[0].clone();
    let r = &f.applied.records;
    assert_eq!(
        r.iter().map(|r| r.seq.get()).collect::<Vec<_>>(),
        [30, 31, 32]
    );
    assert_eq!(r[1].when, When::Scheduled { jitter_ms: 500 });
    assert_eq!((r[1].after_ms, r[1].at_ms), (10_000, Some(7_210_000)));
    assert_eq!(r[2].when, When::Measured);
    assert_eq!((r[2].after_ms, r[2].at_ms), (20_750, Some(7_220_750)));
    let reading = f.reading().unwrap();
    assert_eq!(reading["records"][1]["time"], "scheduled");
    assert_eq!(reading["records"][1]["jitter_ms"], 500);
    assert_eq!(*level.refused.lock().unwrap(), ["frame"]);
    assert!(level.frames.lock().unwrap().is_empty());
}

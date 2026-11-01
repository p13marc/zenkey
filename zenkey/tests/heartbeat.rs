//! A consumer reads heartbeats (core §7.5's form 6; #978's F2): no rule
//! adopts the heartbeat yet and no owner sends one, so these tests write
//! one by hand, raw, on the key F3's `freshness.v2` will declare, from the
//! member's own in-band stamp.
//!
//! Setup: R1 a router; the owner `lab/beacon` implements `beacon.v1` as a
//! client of R1, its status's re-puts off; S a tool's consumer of it on
//! another client, subscribed to `state/status` (a collection: a state
//! with `freshness.ttl_s` = 2).

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{client, config, contract, eventually, imp, router};
use zenkey::ServiceBuilder;
use zenkey::consumer::{Confirmation, Consumer, Delivery};
use zenkey::model::freshness::{Horizon, Reason, Verdict, judge};
use zenkey::model::grammar::IfaceId;
use zenkey::model::heartbeat::{Digest, Heartbeat, Layout, Members, Section, Time};
use zenkey::model::template::Bindings;
use zenkey::model::wire::{Run, Seq, Stamp, Tag};

const STATUS: &str = "state/status";
const KEY: &str = "zk2/lab/beacon/beacon.v1/state/status";

fn beacon() -> IfaceId {
    "beacon.v1".parse().expect("an interface")
}

/// `beacon.v1`'s heartbeat body with the status's section, `digest` over
/// what the owner holds, vouched for or not.
fn heartbeat(layout: &Layout, digest: Digest, vouched: bool, seq: u32) -> Vec<u8> {
    let sections = layout
        .collections
        .iter()
        .map(|c| {
            (c.resource() == STATUS).then(|| Section {
                digest,
                members: Members::One { vouched },
            })
        })
        .collect();
    Heartbeat {
        time: Time::Unknown,
        sections,
    }
    .envelope(
        layout,
        Stamp {
            run: Run::U16(9),
            seq: Seq::new(seq).unwrap(),
        },
    )
    .unwrap()
}

/// A hand-written heartbeat whose digest is the subscription's view
/// confirms the member, so a value put once is fresh past its horizon; one
/// whose digest is not confirms nothing, and two in a row leave the
/// collection due a read; another revision's tag is not read.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_subscription_folds_a_heartbeat_into_its_freshness() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let mut b = ServiceBuilder::new(&os, config("lab/beacon"));
    b.implement(imp("beacon.v1")).unwrap();
    let status = b
        .declare_state_writer(&beacon(), STATUS, &Bindings::new())
        .await
        .unwrap();
    status.set_refresh(false);
    let _svc = b.start().await.unwrap();

    let c = Consumer::for_tool(
        &rs,
        Arc::new(contract("beacon.v1")),
        &["lab/beacon"],
        &Default::default(),
    )
    .unwrap();
    let delivered: Arc<Mutex<Vec<Stamp>>> = Arc::default();
    let confirmed: Arc<Mutex<Vec<Confirmation>>> = Arc::default();
    let (d, k) = (Arc::clone(&delivered), Arc::clone(&confirmed));
    let sub = c
        .subscribe_confirmed(
            STATUS,
            move |x: Delivery| d.lock().unwrap().extend(x.inband),
            move |x: Confirmation| k.lock().unwrap().push(x),
        )
        .await
        .unwrap();
    eventually("the status is matched", || async {
        status.writer().matching().await.unwrap()
    })
    .await;

    // The one put: S holds it under its in-band stamp.
    status.put("up").await.unwrap();
    let put_at = Instant::now();
    eventually("the put is delivered", || async {
        !delivered.lock().unwrap().is_empty()
    })
    .await;
    let stamp = delivered.lock().unwrap()[0];
    let layout = Layout::of(&contract("beacon.v1"));
    assert_eq!(
        layout
            .collections
            .iter()
            .map(|c| c.resource())
            .collect::<Vec<_>>(),
        [STATUS],
        "beacon.v1's one collection: intent's ttl is 0, note has none, level is a stream"
    );
    let hb_key = zenkey::model::heartbeat::key("lab", "beacon", &beacon());
    let h = Horizon::Within(Duration::from_secs(2));
    let judged = || judge(&h, &sub.freshness(KEY));

    // 1.5 s after the put, a heartbeat over what the owner holds.
    tokio::time::sleep_until((put_at + Duration::from_millis(1500)).into()).await;
    let held = Digest::of([(KEY, stamp)]);
    os.put(&hb_key, heartbeat(&layout, held, true, 0))
        .await
        .unwrap();
    eventually("the heartbeat confirms the status", || async {
        !confirmed.lock().unwrap().is_empty()
    })
    .await;
    {
        let c = &confirmed.lock().unwrap()[0];
        assert_eq!(c.keys, [KEY]);
        assert_eq!(c.provider.to_string(), "lab/beacon");
        assert_eq!(c.heartbeat.seq.get(), 0);
        assert_eq!(c.owner_time, Some(Time::Unknown));
    }
    assert_eq!(sub.heartbeats(), 1);
    assert!(sub.confirmed(KEY).is_some());

    // 2.5 s after the put, past its horizon: confirmed 1 s ago, fresh.
    tokio::time::sleep_until((put_at + Duration::from_millis(2500)).into()).await;
    let j = judged();
    assert_eq!(
        (j.verdict, j.reason),
        (Verdict::Fresh, Reason::WithinHorizon)
    );

    // Another revision's tag: not read, counted.
    let mut other = heartbeat(&layout, held, true, 1);
    other[0] = (other[0] & 0xf0) | ((layout.tag.get() + 1) % 16);
    assert_ne!(Tag::new(other[0] & 0x0f), Some(layout.tag));
    os.put(&hb_key, other).await.unwrap();
    eventually("another revision's heartbeat is unread", || async {
        sub.heartbeats_unread() == 1
    })
    .await;

    // A digest the view does not hold (a change it missed): nothing
    // confirmed; a second in a row leaves the collection due a read.
    let missed = Digest::of([(
        KEY,
        Stamp {
            seq: Seq::new(stamp.seq.get() + 1).unwrap(),
            ..stamp
        },
    )]);
    let provider = "lab/beacon".parse().unwrap();
    for seq in [2, 3] {
        os.put(&hb_key, heartbeat(&layout, missed, true, seq))
            .await
            .unwrap();
        eventually("the diverging heartbeat is folded", || async {
            sub.heartbeats() == u64::from(seq)
        })
        .await;
        assert_eq!(sub.reread_due(&provider), seq == 3);
        // 0 then 2: a heartbeat lost, the observation incomplete until 3
        // comes in sequence.
        let complete = matches!(
            sub.freshness(KEY),
            zenkey::model::freshness::Observation::Subscribed { complete: true, .. }
        );
        assert_eq!(complete, seq == 3, "seq {seq}");
    }
    assert_eq!(
        confirmed.lock().unwrap().len(),
        1,
        "a divergence confirms nothing"
    );

    // The heartbeat key's seq skipped from 0 to 2, a heartbeat lost, and 3
    // came in sequence: complete again. At 3.6 s the last confirmation is
    // 2.1 s old.
    tokio::time::sleep_until((put_at + Duration::from_millis(3600)).into()).await;
    let j = judged();
    assert_eq!(
        (j.verdict, j.reason),
        (Verdict::Stale, Reason::BeyondHorizon)
    );
}

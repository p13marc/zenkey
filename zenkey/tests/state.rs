//! `spec/scenarios/state.md` §1–§9 (spec §4, §2.6).
//!
//! Common setup: an owner `ground/fleet-mgr` with the state template
//! `plans/{vehicle}`; a consumer `vehicle-01/executor` bound with
//! `{vehicle} = self.system`. Routers R1 (ground) and R2 (vehicle) are
//! linked through a [`common::Link`] the archive scenarios cut and heal.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use common::{T, client, config, contract, eventually, imp, link_to, router, router_via};
use zenkey::archive::{self, Archive, ArchiveConfig, Recorded};
use zenkey::consumer::Consumer;
use zenkey::model::grammar::IfaceId;
use zenkey::model::template::Bindings;
use zenkey::state::{Current, StateGet, StateWriter, ValueOrder};
use zenkey::{Service, ServiceBuilder};
use zenoh::Wait;
use zenoh::bytes::Encoding;
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::sample::Sample;

/// A union storage on `zk2/*/*/*/events/**` whose backend ignores `_time`,
/// like the storage manager's memory backend (spike S5).
async fn union_storage(
    s: &zenoh::Session,
) -> (
    zenoh::pubsub::Subscriber<()>,
    zenoh::query::Queryable<()>,
    Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
) {
    let store: Arc<Mutex<BTreeMap<String, Vec<u8>>>> = Arc::default();
    let w = Arc::clone(&store);
    let sub = s
        .declare_subscriber("zk2/*/*/*/events/**")
        .callback(move |smp: Sample| {
            w.lock().unwrap().insert(
                smp.key_expr().as_str().to_owned(),
                smp.payload().to_bytes().into_owned(),
            );
        })
        .await
        .unwrap();
    let r = Arc::clone(&store);
    let q = s
        .declare_queryable("zk2/*/*/*/events/**")
        .callback(move |q| {
            for (k, v) in r.lock().unwrap().iter() {
                let ke = OwnedKeyExpr::try_from(k.clone()).unwrap();
                if q.key_expr().intersects(&ke) {
                    let _ = q.reply(ke, v.clone()).encoding(Encoding::TEXT_PLAIN).wait();
                }
            }
        })
        .await
        .unwrap();
    (sub, q, store)
}

/// §8: 1,000 occurrences, 500 inside the retention; a replay bounded by the
/// retention returns exactly the 500, filtering by ULID time because the
/// backend ignores `_time`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s8_events_replay() {
    let (_r1, ep) = router(None).await;
    let owner = client(&ep).await;
    let storage = client(&ep).await;
    let reader = client(&ep).await;
    let journal: IfaceId = "journal.v1".parse().unwrap();
    let (_ssub, _sq, store) = union_storage(&storage).await;

    let mut b = ServiceBuilder::new(&owner, config("lab/journal"));
    b.implement(imp("journal.v1")).unwrap();
    let events = b
        .event_writer(&journal, "events/entries", &Bindings::new())
        .unwrap();
    let _svc = b.start().await.unwrap();
    // The storage's subscriber must be known before the puts.
    let probe = owner
        .declare_publisher("zk2/lab/journal/journal.v1/events/entries/x")
        .await
        .unwrap();
    eventually("the storage subscribes", || async {
        probe.matching_status().await.unwrap().matching()
    })
    .await;
    drop(probe);
    let now = SystemTime::now();
    for i in 0..500u64 {
        events
            .put_at(format!("new{i}"), now - Duration::from_secs(i))
            .await
            .unwrap();
        events
            .put_at(format!("old{i}"), now - Duration::from_secs(7200 + i))
            .await
            .unwrap();
    }
    eventually("the storage holds 1,000", || async {
        store.lock().unwrap().len() == 1000
    })
    .await;

    let mut b = ServiceBuilder::new(
        &reader,
        config("lab/auditor").bind("journal", &["lab/journal"]),
    );
    b.require("journal", journal, false);
    let auditor = b.start().await.unwrap();
    let consumer = auditor
        .consumer("journal", Arc::new(contract("journal.v1")))
        .unwrap();
    let got = consumer
        .replay("events/entries", Duration::from_secs(3600), T)
        .await
        .unwrap();
    assert_eq!(got.len(), 500, "exactly the recent ones");
    assert!(
        got.iter()
            .all(|d| d.sample.payload().to_bytes().starts_with(b"new"))
    );
}

fn plan() -> IfaceId {
    "mission_plan.v1".parse().unwrap()
}

fn vehicle(v: &str) -> Bindings {
    [("vehicle".to_owned(), vec![v.to_owned()])].into()
}

const ORIGIN: &str = "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01";

/// The owner, serving `plans/{vehicle}`, with writers for `vehicles`.
async fn fleet_mgr(
    s: &zenoh::Session,
    window_s: Option<u64>,
    vehicles: &[&str],
) -> (Service, BTreeMap<String, StateWriter>) {
    let mut cfg = config("ground/fleet-mgr");
    cfg.tombstone_window_s = window_s;
    let mut b = ServiceBuilder::new(s, cfg);
    b.implement(imp("mission_plan.v1")).unwrap();
    b.expose(&plan(), "state/plans/{vehicle}").unwrap();
    b.serve_state(&plan()).unwrap();
    let mut svc = b.start().await.unwrap();
    let mut w = BTreeMap::new();
    for v in vehicles {
        w.insert(
            (*v).to_owned(),
            svc.state_writer(&plan(), "state/plans/{vehicle}", &vehicle(v))
                .await
                .unwrap(),
        );
    }
    (svc, w)
}

/// A consumer of `plan`, bound to the fleet manager, with `{vehicle} =
/// self.system` when `own_slice`.
async fn executor(s: &zenoh::Session, own_slice: bool) -> (Service, Consumer) {
    let mut cfg = config("vehicle-01/executor").bind("plan", &["ground/fleet-mgr"]);
    if own_slice {
        cfg.bindings
            .get_mut("plan")
            .unwrap()
            .params
            .insert("vehicle".to_owned(), "self.system".to_owned());
    }
    let mut b = ServiceBuilder::new(s, cfg);
    b.require("plan", plan(), false);
    let svc = b.start().await.unwrap();
    let c = svc
        .consumer("plan", Arc::new(contract("mission_plan.v1")))
        .unwrap();
    (svc, c)
}

async fn answered(c: &Consumer, values: Option<&Bindings>) -> Vec<Current> {
    match c.get("state/plans/{vehicle}", values, T).await.unwrap() {
        StateGet::Answered(v) => v,
        StateGet::Silent => panic!("the owner did not answer"),
    }
}

/// §1: the owner and the consumer are clients of R1, which stamps. Every
/// mutation, the delete included, carries the owner's stamp, increasing; a
/// GET after v2 returns v2 with v2's stamp. The control: an unstamped put
/// through R1 carries R1's zid, so the check tells the two apart (core
/// §4.2, "Observing S1", F-69). Two puts back to back step by at least one
/// tick (§4.3, F-66).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_stamped_mutations() {
    let (r1, ep) = router(None).await;
    let owner = client(&ep).await;
    let (_mgr, w) = fleet_mgr(&owner, None, &["vehicle-01"]).await;
    let consumer = client(&ep).await;
    let (_ex, c) = executor(&consumer, true).await;
    let got: Arc<Mutex<Vec<Sample>>> = Arc::default();
    let g = Arc::clone(&got);
    let _sub = c
        .subscribe("state/plans/{vehicle}", move |d| {
            g.lock().unwrap().push(d.sample)
        })
        .await
        .unwrap();
    let w = &w["vehicle-01"];
    eventually("subscribed", || async {
        w.writer().matching().await.unwrap()
    })
    .await;
    let t1 = w.put("v1").await.unwrap();
    let t2 = w.put("v2").await.unwrap();
    let now = answered(&c, None).await;
    assert_eq!(now.len(), 1);
    assert!(
        matches!(&now[0], Current::Value { sample, .. } if &*sample.payload().to_bytes() == b"v2")
    );
    assert_eq!(now[0].timestamp(), Some(t2), "v2 with v2's stamp");
    let t3 = w.delete().await.unwrap();
    eventually("three mutations", || async {
        got.lock().unwrap().len() == 3
    })
    .await;
    {
        let got = got.lock().unwrap();
        let stamps: Vec<_> = got
            .iter()
            .map(|s| *s.timestamp().expect("stamped"))
            .collect();
        assert_eq!(stamps, [t1, t2, t3]);
        assert!(stamps.windows(2).all(|p| p[0] < p[1]));
        let zid = owner.zid().to_string();
        assert_ne!(zid, r1.zid().to_string(), "the owner is not R1");
        assert!(
            stamps.iter().all(|t| t.get_id().to_string() == zid),
            "the owner's zid, not a router's"
        );
        assert_eq!(got[2].kind(), zenoh::sample::SampleKind::Delete);
    }

    // 2. The control: a third client puts without a timestamp; R1 stamps it.
    let third = client(&ep).await;
    let control = consumer
        .declare_subscriber("control/s1")
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + common::SETTLE;
    let sample = loop {
        third.put("control/s1", "x").await.unwrap();
        if let Ok(Ok(s)) =
            tokio::time::timeout(Duration::from_millis(100), control.recv_async()).await
        {
            break s;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "never: the control's put"
        );
    };
    let stamp = sample.timestamp().expect("R1 stamps an unstamped put");
    assert_eq!(stamp.get_id().to_string(), r1.zid().to_string());
    assert_ne!(stamp.get_id().to_string(), third.zid().to_string());

    // 3. v3 and v4 back to back. Whether they fell within one clock
    //    reading cannot be seen from here (F-73); the order can.
    let t3 = w.put("v3").await.unwrap();
    let t4 = w.put("v4").await.unwrap();
    assert!(
        t4.get_time().as_u64() > t3.get_time().as_u64(),
        "at least one tick (one NTP64 unit) apart"
    );
    assert_eq!(t4.get_id(), t3.get_id());
}

/// §2: within the window, a deleted key answers `reply_del` with the
/// deletion's stamp, alone and inside the collection.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_deletes_inside_the_window_and_the_collection() {
    let (_r1, ep) = router(None).await;
    let (_mgr, w) = fleet_mgr(&client(&ep).await, None, &["a", "b"]).await;
    let (_ex, c) = executor(&client(&ep).await, false).await;
    w["a"].put("pa").await.unwrap();
    w["b"].put("pb").await.unwrap();
    let del = w["b"].delete().await.unwrap();
    let b = answered(&c, Some(&vehicle("b"))).await;
    assert!(
        matches!(&b[..], [Current::Deleted { timestamp: Some(t), .. }] if *t == del),
        "{b:?}"
    );
    let all = answered(&c, None).await;
    let by_key: BTreeMap<&str, &Current> = all.iter().map(|c| (c.key(), c)).collect();
    assert_eq!(by_key.len(), 2);
    assert!(matches!(
        by_key["zk2/ground/fleet-mgr/mission_plan.v1/state/plans/a"],
        Current::Value { .. }
    ));
    assert!(matches!(
        by_key["zk2/ground/fleet-mgr/mission_plan.v1/state/plans/b"],
        Current::Deleted { .. }
    ));
}

/// §3 (step 2): the consumer's GET (`All` + `Latest`, set explicitly by
/// `Consumer::get`) is answered by the owner alone. Step 1, the storage
/// finding read from the routers' admin space, is a tool's (`zenctl
/// doctor`, #612).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_the_owner_is_authoritative() {
    let (_r1, ep) = router(None).await;
    let owner = client(&ep).await;
    let (_mgr, w) = fleet_mgr(&owner, None, &["vehicle-01"]).await;
    let (_ex, c) = executor(&client(&ep).await, true).await;
    let t = w["vehicle-01"].put("mine").await.unwrap();
    let now = answered(&c, None).await;
    assert_eq!(now.len(), 1, "one answer: the owner's");
    assert_eq!(now[0].timestamp(), Some(t));
    assert_eq!(t.get_id().to_string(), owner.zid().to_string());
}

/// Two sites: the ground (R1) with the owner, the vehicle (R2) with its
/// archive and the executor, linked by a cuttable link.
struct Sites {
    link: common::Link,
    ground: zenoh::Session,
    vehicle: zenoh::Session,
    _routers: (zenoh::Session, zenoh::Session),
}

async fn sites() -> Sites {
    let (r1, ep1) = router(None).await;
    let link = link_to(&ep1).await;
    let (r2, ep2) = router_via(&link).await;
    Sites {
        ground: client(&ep1).await,
        vehicle: client(&ep2).await,
        link,
        _routers: (r1, r2),
    }
}

fn recorded(selector: &str) -> Recorded {
    Recorded {
        owner: common::addr("ground/fleet-mgr"),
        selector: selector.to_owned(),
        implementation: imp("mission_plan.v1"),
    }
}

async fn vehicle_archive(s: &zenoh::Session, peers: &[&str]) -> Archive {
    Archive::start(
        s,
        ArchiveConfig {
            service: config("vehicle-01/archive"),
            records: vec![recorded(ORIGIN)],
            peers: peers.iter().map(|p| common::addr(p)).collect(),
            unconfirmed_horizon: None,
        },
    )
    .await
    .unwrap()
}

async fn wait_owner_seen(s: &zenoh::Session, present: bool) {
    eventually(
        if present {
            "the owner is visible"
        } else {
            "the owner is gone"
        },
        || async {
            zenkey::presence::liveliness_keys(s, "zk2/ground/fleet-mgr/@zk/instance/*", T)
                .await
                .unwrap()
                .is_empty()
                != present
        },
    )
    .await;
}

/// §4: with the link cut, the owner is silent; the archive gives v3 with
/// its stamp and type identity, as last-known.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_last_known_from_an_archive() {
    let x = sites().await;
    let (_mgr, w) = fleet_mgr(&x.ground, None, &["vehicle-01"]).await;
    let arch = vehicle_archive(&x.vehicle, &[]).await;
    let (_ex, c) = executor(&x.vehicle, true).await;
    wait_owner_seen(&x.vehicle, true).await;
    eventually("the archive records", || async {
        w["vehicle-01"].put("v2").await.unwrap();
        arch.confirmed(ORIGIN) == Some(true)
    })
    .await;
    // v3, once, and the archive holding exactly it before the cut.
    let t3 = Some(w["vehicle-01"].put("v3").await.unwrap());
    eventually("the archive recorded v3", || async {
        archive::last_known(&x.vehicle, &common::addr("vehicle-01/archive"), ORIGIN, T)
            .await
            .unwrap()
            .is_some_and(|l| l.timestamp == t3)
    })
    .await;
    x.link.cut();
    wait_owner_seen(&x.vehicle, false).await;
    assert!(matches!(
        c.get("state/plans/{vehicle}", None, T).await.unwrap(),
        StateGet::Silent
    ));
    let lk = archive::last_known(&x.vehicle, &common::addr("vehicle-01/archive"), ORIGIN, T)
        .await
        .unwrap()
        .expect("the archive answers");
    assert_eq!(lk.value.as_deref(), Some(&b"v3"[..]));
    assert_eq!(lk.timestamp, t3);
    assert_eq!(lk.identity["iface"], "mission_plan.v1");
    assert_eq!(
        lk.identity["contract"],
        zenkey::Implementation::new(contract("mission_plan.v1"))
            .fingerprint()
            .to_string()
    );
    assert_eq!(lk.identity["type"]["kind"], "raw");
    // The same, over a pattern (#671): every origin the archive holds that
    // the pattern selects, here the one it recorded.
    let all = archive::last_known_all(
        &x.vehicle,
        &common::addr("vehicle-01/archive"),
        "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/*",
        T,
    )
    .await
    .unwrap();
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0].origin, ORIGIN);
    assert_eq!(all[0].value.as_deref(), Some(&b"v3"[..]));
    assert_eq!(all[0].timestamp, t3);
}

/// §5: alignment after reconnect drops a key only on a `reply_del`, keeps
/// it unconfirmed on an empty reply set, and aligns from an owner-side
/// archive while the owner is gone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_alignment_after_reconnect() {
    let lk = |s: &zenoh::Session| {
        let s = s.clone();
        async move {
            archive::last_known(&s, &common::addr("vehicle-01/archive"), ORIGIN, T)
                .await
                .unwrap()
        }
    };
    // 1. Deleted during the cut, within W: dropped after the heal.
    {
        let x = sites().await;
        let (_mgr, w) = fleet_mgr(&x.ground, None, &["vehicle-01"]).await;
        let arch = vehicle_archive(&x.vehicle, &[]).await;
        wait_owner_seen(&x.vehicle, true).await;
        eventually("recorded", || async {
            w["vehicle-01"].put("p").await.unwrap();
            arch.confirmed(ORIGIN) == Some(true)
        })
        .await;
        x.link.cut();
        wait_owner_seen(&x.vehicle, false).await;
        w["vehicle-01"].delete().await.unwrap();
        x.link.heal();
        eventually("dropped on the owner's reply_del", || async {
            lk(&x.vehicle).await.is_some_and(|l| l.value.is_none())
        })
        .await;
    }
    // 2. The same, but the archive's read is refused once: nothing dropped,
    //    served unconfirmed.
    {
        let x = sites().await;
        let (_mgr, w) = fleet_mgr(&x.ground, None, &["vehicle-01"]).await;
        let arch = vehicle_archive(&x.vehicle, &[]).await;
        wait_owner_seen(&x.vehicle, true).await;
        eventually("recorded", || async {
            w["vehicle-01"].put("p").await.unwrap();
            arch.confirmed(ORIGIN) == Some(true)
        })
        .await;
        x.link.cut();
        wait_owner_seen(&x.vehicle, false).await;
        w["vehicle-01"].delete().await.unwrap();
        // The access control refuses the archive's reads, its retries included.
        arch.refuse_next_reads(u32::MAX);
        x.link.heal();
        eventually("kept, unconfirmed", || async {
            arch.confirmed(ORIGIN) == Some(false)
        })
        .await;
        let l = lk(&x.vehicle).await.expect("still served");
        assert_eq!(l.value.as_deref(), Some(&b"p"[..]));
        assert!(!l.confirmed);
    }
    // 3. The owner is gone; an archive on its side recorded the delete.
    {
        let x = sites().await;
        let (mgr, w) = fleet_mgr(&x.ground, None, &["vehicle-01"]).await;
        let _ground_arch = Archive::start(
            &x.ground,
            ArchiveConfig {
                service: config("ground/archive"),
                records: vec![recorded(
                    "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/*",
                )],
                peers: vec![],
                unconfirmed_horizon: None,
            },
        )
        .await
        .unwrap();
        let arch = vehicle_archive(&x.vehicle, &["ground/archive"]).await;
        wait_owner_seen(&x.vehicle, true).await;
        eventually("recorded", || async {
            w["vehicle-01"].put("p").await.unwrap();
            arch.confirmed(ORIGIN) == Some(true)
        })
        .await;
        x.link.cut();
        wait_owner_seen(&x.vehicle, false).await;
        w["vehicle-01"].delete().await.unwrap();
        drop(w);
        mgr.close().await.unwrap();
        x.link.heal();
        eventually("dropped on the owner-side archive's reply_del", || async {
            lk(&x.vehicle).await.is_some_and(|l| l.value.is_none())
        })
        .await;
    }
}

/// §6: a delete during an outage longer than the window is answered after
/// the heal only when the window covers the outage.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_a_window_shorter_than_the_outage() {
    for (window, dropped) in [(5u64, true), (1u64, false)] {
        let x = sites().await;
        let (_mgr, w) = fleet_mgr(&x.ground, Some(window), &["vehicle-01"]).await;
        let arch = vehicle_archive(&x.vehicle, &[]).await;
        wait_owner_seen(&x.vehicle, true).await;
        eventually("recorded", || async {
            w["vehicle-01"].put("p").await.unwrap();
            arch.confirmed(ORIGIN) == Some(true)
        })
        .await;
        x.link.cut();
        wait_owner_seen(&x.vehicle, false).await;
        w["vehicle-01"].delete().await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        x.link.heal();
        wait_owner_seen(&x.vehicle, true).await;
        let lk = || async {
            archive::last_known(&x.vehicle, &common::addr("vehicle-01/archive"), ORIGIN, T)
                .await
                .unwrap()
                .unwrap()
        };
        if dropped {
            eventually(
                "W covers the outage: dropped on the owner's reply_del",
                || async { lk().await.value.is_none() },
            )
            .await;
        } else {
            eventually("aligned, unconfirmed", || async {
                arch.confirmed(ORIGIN) == Some(false)
            })
            .await;
            // The retries run their course; no evidence ever comes.
            tokio::time::sleep(Duration::from_secs(4)).await;
            let l = lk().await;
            assert_eq!(
                l.value.as_deref(),
                Some(&b"p"[..]),
                "W = {window}s: no evidence, kept"
            );
            assert!(!l.confirmed);
        }
    }
}

/// A session with a pinned zid, as an owner that must catch up uses.
async fn pinned(ep: &str, id: &str) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{ep}\"]"))
        .unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("id", &format!("\"{id}\"")).unwrap();
    zenoh::open(c).await.unwrap()
}

/// §7: catch-up keeps one id's stamps increasing across a restart with the
/// clock behind; a new epoch is accepted under its new id; an owner ahead of
/// its router stops writing state.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_clocks() {
    let (_r1, ep) = router(None).await;
    let key = ORIGIN;
    // 1. Catch-up: rev 10, a restart 5 s behind, the record read, rev 11.
    let mut order = ValueOrder::new();
    let (t10, record) = {
        let s = pinned(&ep, "a1b2c3d4e5f60718").await;
        let (mgr, w) = fleet_mgr(&s, None, &["vehicle-01"]).await;
        let t = w["vehicle-01"].put("rev10").await.unwrap();
        let rec = mgr.minter().last().unwrap();
        drop(w);
        mgr.close().await.unwrap();
        s.close().await.unwrap();
        (t, rec)
    };
    assert!(order.accept(key, &t10));
    let t11 = {
        let s = pinned(&ep, "a1b2c3d4e5f60718").await;
        let (mgr, w) = fleet_mgr(&s, None, &["vehicle-01"]).await;
        mgr.minter().simulate_offset(-5000);
        mgr.minter().catch_up(record);
        let t = w["vehicle-01"].put("rev11").await.unwrap();
        drop(w);
        mgr.close().await.unwrap();
        t
    };
    assert_eq!(t11.get_id(), t10.get_id());
    assert!(t11 > t10, "above rev 10's stamp");
    // The clock reads behind the record, so the mint is the record plus
    // one tick exactly (§4.3): the step state.md §1 cannot show (F-73).
    assert_eq!(
        t11.get_time().as_u64(),
        record.get_time().as_u64() + 1,
        "one NTP64 unit above the record"
    );
    assert!(order.accept(key, &t11), "the consumer applies it");

    // 2. New epoch: no record, a fresh zid, 5 s behind.
    let s = client(&ep).await;
    let (mgr, w) = fleet_mgr(&s, None, &["vehicle-01"]).await;
    mgr.minter().simulate_offset(-5000);
    let t11b = w["vehicle-01"].put("rev11").await.unwrap();
    assert_ne!(t11b.get_id(), t10.get_id(), "a new timestamp id");
    assert!(t11b.get_time() < t11.get_time(), "older by the clock");
    assert!(
        order.accept(key, &t11b),
        "the first value under a new id is accepted"
    );
    drop(w);
    mgr.close().await.unwrap();

    // 3. Ahead: 2 s ahead of the router, which stamps a heartbeat.
    let hb = client(&ep).await;
    let s = client(&ep).await;
    let mut cfg = config("ground/fleet-mgr");
    cfg.clock_reference = Some("clock/heartbeat".to_owned());
    let mut b = ServiceBuilder::new(&s, cfg);
    b.implement(imp("mission_plan.v1")).unwrap();
    b.minter().simulate_offset(2000);
    let w = b
        .declare_state_writer(&plan(), "state/plans/{vehicle}", &vehicle("vehicle-01"))
        .await
        .unwrap();
    let mgr = b.start().await.unwrap();
    eventually("the drift is detected", || async {
        hb.put("clock/heartbeat", "tick").await.unwrap();
        mgr.minter().is_ahead()
    })
    .await;
    assert!(
        matches!(w.put("late").await, Err(zenkey::Error::ClockAhead)),
        "state writes stop"
    );
}

/// §9: an archive that holds a delete refuses an older put arriving late.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s9_an_archives_backend_refuses_an_outdated_put() {
    let (_r1, ep) = router(None).await;
    let owner = client(&ep).await;
    let (_mgr, w) = fleet_mgr(&owner, None, &["vehicle-01"]).await;
    let arch_s = client(&ep).await;
    let arch = vehicle_archive(&arch_s, &[]).await;
    let w = &w["vehicle-01"];
    let t1 = Arc::new(Mutex::new(None));
    eventually("recorded v1", || async {
        *t1.lock().unwrap() = Some(w.put("v1").await.unwrap());
        arch.confirmed(ORIGIN) == Some(true)
    })
    .await;
    let t1 = t1.lock().unwrap().unwrap();
    w.delete().await.unwrap();
    eventually("recorded the delete", || async {
        archive::last_known(&arch_s, &common::addr("vehicle-01/archive"), ORIGIN, T)
            .await
            .unwrap()
            .is_some_and(|l| l.value.is_none())
    })
    .await;
    // An older put (T0 < T2) reaches the archive late.
    let t0 = zenoh::time::Timestamp::new(*t1.get_time() - 1, *t1.get_id());
    owner.put(ORIGIN, "v0").timestamp(t0).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let l = archive::last_known(&arch_s, &common::addr("vehicle-01/archive"), ORIGIN, T)
        .await
        .unwrap()
        .unwrap();
    assert!(l.value.is_none(), "a tombstone, never the older value");
}

/// #698 (§2.3, S2): a state value and an event occurrence carry the
/// attachment their contract declares. The subscriber receives it with the
/// sample, and the owner's GET answers carry it with the value.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn state_and_events_carry_their_attachments() {
    let (_r1, ep) = router(None).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let notes: IfaceId = "notes.v1".parse().unwrap();
    let mut b = ServiceBuilder::new(&owner, config("h1/notes"));
    b.implement(imp("notes.v1")).unwrap();
    let none = Bindings::new();
    let last = b
        .declare_state_writer(&notes, "state/last", &none)
        .await
        .unwrap();
    let posted = b.event_writer(&notes, "events/posted", &none).unwrap();
    let _svc = b.start().await.unwrap();

    // Each sample's payload, and its attachment if any.
    type Seen = Vec<(String, Option<Vec<u8>>)>;
    let seen: Arc<Mutex<Seen>> = Arc::default();
    let s = Arc::clone(&seen);
    let _sub = tool
        .declare_subscriber("zk2/h1/notes/notes.v1/**")
        .callback(move |sample: Sample| {
            s.lock().unwrap().push((
                String::from_utf8_lossy(&sample.payload().to_bytes()).into_owned(),
                sample.attachment().map(|a| a.to_bytes().into_owned()),
            ));
        })
        .await
        .unwrap();
    let probe = owner
        .declare_publisher("zk2/h1/notes/notes.v1/state/last")
        .await
        .unwrap();
    eventually("the tool's subscriber is matched", || async {
        probe.matching_status().await.unwrap().matching()
    })
    .await;
    drop(probe);

    last.put_with("hello", Some("by-alice")).await.unwrap();
    posted.put_with("posted", Some("by-bob")).await.unwrap();
    eventually("both samples arrive", || async {
        seen.lock().unwrap().len() == 2
    })
    .await;
    let got = seen.lock().unwrap().clone();
    assert!(
        got.contains(&("hello".to_owned(), Some(b"by-alice".to_vec()))),
        "{got:?}"
    );
    assert!(
        got.contains(&("posted".to_owned(), Some(b"by-bob".to_vec()))),
        "{got:?}"
    );

    // S2: the owner's GET answers the value with its attachment.
    let rx = tool
        .get("zk2/h1/notes/notes.v1/state/last")
        .target(zenoh::query::QueryTarget::All)
        .with(flume::unbounded::<zenoh::query::Reply>())
        .await
        .unwrap();
    let reply = rx.recv_async().await.expect("an answer");
    let sample = reply.result().expect("a value");
    assert_eq!(&*sample.payload().to_bytes(), b"hello");
    assert_eq!(
        sample.attachment().map(|a| a.to_bytes().into_owned()),
        Some(b"by-alice".to_vec())
    );
}

/// Core §4.4 (0.16): an archive is never tokenless, because consumers and
/// tools find it by its interface token (S6); one configured so is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_archive_is_never_tokenless() {
    let (_r1, ep) = router(None).await;
    let s = client(&ep).await;
    let refused = Archive::start(
        &s,
        ArchiveConfig {
            service: config("vehicle-01/archive").tokenless("archive.v1".parse().unwrap()),
            records: vec![recorded(ORIGIN)],
            peers: Vec::new(),
            unconfirmed_horizon: None,
        },
    )
    .await;
    let Err(e) = refused else {
        panic!("a tokenless archive starts")
    };
    assert!(e.to_string().contains("never tokenless"), "{e}");
}

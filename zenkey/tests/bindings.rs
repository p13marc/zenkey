//! `spec/scenarios/bindings.md` §1–§5 (spec §3.2 R1–R6).

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{SETTLE, T, client, config, contract, eventually, imp, router};
use zenkey::consumer::Delivery;
use zenkey::model::grammar::{IfaceId, ZkKey};
use zenkey::model::template::Bindings;
use zenkey::presence::{self, Edge};
use zenkey::writer::Writer;
use zenkey::{Service, ServiceBuilder};

fn detections() -> IfaceId {
    "detections.v1".parse().unwrap()
}

/// A detector: `detections.v1` with a writer on `objects`.
async fn detector(s: &zenoh::Session, address: &str) -> (Service, Writer) {
    let mut b = ServiceBuilder::new(s, config(address));
    b.implement(imp("detections.v1")).unwrap();
    let w = b
        .declare_writer(&detections(), "stream/objects", &Bindings::new())
        .await
        .unwrap();
    (b.start().await.unwrap(), w)
}

/// A consumer service binding `sources` (a manifest role) to `providers`,
/// counting deliveries per provider.
async fn tracker(
    s: &zenoh::Session,
    address: &str,
    providers: &[&str],
) -> (
    Service,
    zenkey::consumer::Subscription,
    Arc<Mutex<BTreeMap<String, u64>>>,
) {
    let mut b = ServiceBuilder::new(s, config(address).bind("sources", providers));
    b.require("sources", detections(), false);
    let svc = b.start().await.unwrap();
    let got: Arc<Mutex<BTreeMap<String, u64>>> = Arc::default();
    let g = Arc::clone(&got);
    let sub = svc
        .consumer("sources", Arc::new(contract("detections.v1")))
        .unwrap()
        .subscribe("stream/objects", move |d: Delivery| {
            *g.lock().unwrap().entry(d.provider.to_string()).or_default() += 1;
        })
        .await
        .unwrap();
    (svc, sub, got)
}

async fn put_n(w: &Writer, n: usize) {
    eventually("a subscriber matches", || async {
        w.matching().await.unwrap()
    })
    .await;
    for i in 0..n {
        w.put(format!("d{i}")).await.unwrap();
    }
}

/// §1: the binding resolves before any provider exists; every running
/// detector's samples are delivered; a presence wait returns within one
/// token propagation of the first provider's arrival.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_static_bindings_and_fan_in() {
    const N: usize = 100;
    let (_r1, ep) = router(None).await;
    let tracker_s = client(&ep).await;
    // 1. The tracker first: the binding resolves at once, without presence.
    let t0 = Instant::now();
    let (t_svc, _sub, got) = tracker(&tracker_s, "ground/tracker", &["vehicle-01/*"]).await;
    assert!(
        t0.elapsed() < Duration::from_secs(2),
        "resolved without waiting"
    );
    let waiter = t_svc
        .consumer("sources", Arc::new(contract("detections.v1")))
        .unwrap();
    let waiting = tokio::spawn(async move {
        let found = waiter.wait_for_provider(SETTLE).await.unwrap();
        (found, Instant::now())
    });

    // 2. Detectors, one by one, on ten sessions.
    let mut sessions = Vec::new();
    for _ in 0..10 {
        sessions.push(client(&ep).await);
    }
    let mut dets = Vec::new();
    let mut first_up = None;
    for i in 0..N {
        let d = detector(&sessions[i % 10], &format!("vehicle-01/det{i}")).await;
        first_up.get_or_insert_with(Instant::now);
        dets.push(d);
    }
    let (found, at) = waiting.await.unwrap();
    assert!(found.is_some(), "the waiter found a provider");
    assert!(
        at.saturating_duration_since(first_up.unwrap()) < Duration::from_secs(1),
        "within one token propagation"
    );
    for (_, w) in &dets {
        put_n(w, 5).await;
    }
    eventually("five from every detector", || async {
        let g = got.lock().unwrap();
        g.len() == N && g.values().all(|n| *n == 5)
    })
    .await;

    // Half stop; the rest keep delivering.
    for (svc, _) in dets.drain(..N / 2) {
        svc.close().await.unwrap();
    }
    for (_, w) in &dets {
        put_n(w, 5).await;
    }
    eventually("ten from every running detector", || async {
        let g = got.lock().unwrap();
        dets.iter()
            .all(|(s, _)| g.get(&s.address().to_string()) == Some(&10))
    })
    .await;
}

/// §2: `{vehicle} = self.system` narrows the role to the consumer's own
/// slice of the provider's collection.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_binding_a_parameter_to_self() {
    let (_r1, ep) = router(None).await;
    let ground = client(&ep).await;
    let vehicle = client(&ep).await;
    let plan: IfaceId = "mission_plan.v1".parse().unwrap();

    let mut b = ServiceBuilder::new(&ground, config("ground/fleet-mgr"));
    b.implement(imp("mission_plan.v1")).unwrap();
    let mut writers = Vec::new();
    for v in ["vehicle-01", "vehicle-02"] {
        let vals: Bindings = [("vehicle".to_owned(), vec![v.to_owned()])].into();
        writers.push(
            b.declare_writer(&plan, "state/plans/{vehicle}", &vals)
                .await
                .unwrap(),
        );
    }
    let _mgr = b.start().await.unwrap();

    let mut cfg = config("vehicle-01/executor").bind("plan", &["ground/fleet-mgr"]);
    cfg.bindings
        .get_mut("plan")
        .unwrap()
        .params
        .insert("vehicle".to_owned(), "self.system".to_owned());
    let mut b = ServiceBuilder::new(&vehicle, cfg);
    b.require("plan", plan.clone(), false);
    let exec = b.start().await.unwrap();
    let consumer = exec
        .consumer("plan", Arc::new(contract("mission_plan.v1")))
        .unwrap();
    let sel: Vec<String> = consumer
        .selectors("state/plans/{vehicle}")
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        sel,
        ["zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01"]
    );

    let keys: Arc<Mutex<BTreeSet<String>>> = Arc::default();
    let k = Arc::clone(&keys);
    let _sub = consumer
        .subscribe("state/plans/{vehicle}", move |d: Delivery| {
            k.lock()
                .unwrap()
                .insert(d.sample.key_expr().as_str().to_owned());
        })
        .await
        .unwrap();
    eventually("the executor's slice is matched", || async {
        writers[0].matching().await.unwrap()
    })
    .await;
    for w in &writers {
        w.put("go").await.unwrap();
    }
    eventually("its plan arrived", || async {
        !keys.lock().unwrap().is_empty()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        *keys.lock().unwrap(),
        BTreeSet::from(["zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01".to_owned()])
    );
    assert!(
        !writers[1].matching().await.unwrap(),
        "nothing subscribes to another vehicle's plan"
    );
}

/// §3: the graph from descriptors and interface tokens alone matches the
/// deliveries edge for edge.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_the_graph_from_descriptors() {
    let (_r1, ep) = router(None).await;
    let dets_s = client(&ep).await;
    let trackers_s = client(&ep).await;
    let tool = client(&ep).await;
    let mut dets = Vec::new();
    for sys in ["vehicle-01", "vehicle-02"] {
        for i in 0..3 {
            dets.push(detector(&dets_s, &format!("{sys}/det{i}")).await);
        }
    }
    let mut trackers = Vec::new();
    for (name, providers) in [
        ("ground/all-01", &["vehicle-01/*"][..]),
        ("ground/one", &["vehicle-02/det1"][..]),
        ("ground/mixed", &["vehicle-01/det0", "vehicle-02/*"][..]),
    ] {
        trackers.push((name, tracker(&trackers_s, name, providers).await));
    }
    // Deliveries, observed: 3 + 1 + 4 bound providers. Re-put until every
    // edge has delivered, since a detector's first match can be one tracker
    // while another's subscription is still propagating.
    eventually("every edge delivered", || async {
        for (_, w) in &dets {
            w.put("d").await.unwrap();
        }
        trackers
            .iter()
            .map(|(_, (_, _, g))| g.lock().unwrap().len())
            .sum::<usize>()
            == 8
    })
    .await;
    let delivered: BTreeSet<Edge> = trackers
        .iter()
        .flat_map(|(name, (_, _, g))| {
            g.lock()
                .unwrap()
                .keys()
                .map(|p| Edge {
                    consumer: (*name).to_owned(),
                    role: "sources".to_owned(),
                    provider: p.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect();

    // The graph, read.
    let tokens = presence::tokens(&tool, "zk2/*/*/@zk/**", T).await.unwrap();
    let mut descriptors = Vec::new();
    for t in &tokens {
        if let ZkKey::Instance { addr, instance } = t {
            descriptors.push(
                presence::descriptor(&tool, addr, instance, T)
                    .await
                    .unwrap()
                    .into_descriptor()
                    .unwrap(),
            );
        }
    }
    let drawn: BTreeSet<Edge> = presence::edges(&descriptors, &tokens).into_iter().collect();
    assert_eq!(drawn.len(), 8);
    assert_eq!(drawn, delivered);
}

/// §4: puts on a wildcard key reach a bound consumer with that key, and are
/// discarded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_wildcard_puts() {
    let (_r1, ep) = router(None).await;
    let s = client(&ep).await;
    let principal = client(&ep).await;
    let (_svc, sub, got) = tracker(&s, "ground/tracker", &["vehicle-01/*"]).await;
    let p = principal
        .declare_publisher("zk2/vehicle-01/*/detections.v1/stream/objects")
        .await
        .unwrap();
    eventually("the consumer is matched", || async {
        p.matching_status().await.unwrap().matching()
    })
    .await;
    for i in 0..10 {
        p.put(format!("w{i}")).await.unwrap();
    }
    eventually("all ten discarded", || async { sub.discarded() == 10 }).await;
    assert!(got.lock().unwrap().is_empty(), "none delivered");
}

/// §5 (0.20): `self.system/<service>` and `self.system/*` name providers on
/// the consumer's own system, resolved once at start. The descriptor lists
/// them resolved; a tool, with no system of its own, is refused one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_provider_on_the_services_own_system() {
    let (_r1, ep) = router(None).await;
    let dets_s = client(&ep).await;
    let trackers_s = client(&ep).await;
    let tool = client(&ep).await;
    let mut dets = Vec::new();
    for address in ["vehicle-01/det0", "vehicle-01/det1", "vehicle-02/det0"] {
        dets.push(detector(&dets_s, address).await);
    }
    let one = tracker(&trackers_s, "vehicle-01/one", &["self.system/det0"]).await;
    let all = tracker(&trackers_s, "vehicle-01/all", &["self.system/*"]).await;

    // 1. Delivered from the own system only: det0 to `one`, both of
    // vehicle-01's to `all`. Re-put until every edge has delivered.
    eventually("every bound provider delivered", || async {
        for (_, w) in &dets {
            w.put("d").await.unwrap();
        }
        one.2.lock().unwrap().len() == 1 && all.2.lock().unwrap().len() == 2
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let delivered = |g: &Arc<Mutex<BTreeMap<String, u64>>>| -> BTreeSet<String> {
        g.lock().unwrap().keys().cloned().collect()
    };
    assert_eq!(
        delivered(&one.2),
        BTreeSet::from(["vehicle-01/det0".to_owned()])
    );
    assert_eq!(
        delivered(&all.2),
        BTreeSet::from(["vehicle-01/det0".to_owned(), "vehicle-01/det1".to_owned()])
    );

    // 2. The descriptors list the bindings resolved (R3, D009).
    for (svc, want) in [(&one.0, "vehicle-01/det0"), (&all.0, "vehicle-01/*")] {
        let d = presence::descriptor(&tool, svc.address(), svc.instance(), T)
            .await
            .unwrap()
            .into_descriptor()
            .unwrap();
        assert_eq!(d.requires[0].bindings, [want]);
    }

    // 3. A tool has no system of its own.
    let refused = zenkey::consumer::Consumer::for_tool(
        &tool,
        Arc::new(contract("detections.v1")),
        &["self.system/det0"],
        &BTreeMap::new(),
    );
    assert!(
        refused.is_err_and(|e| e.to_string().contains("own system")),
        "a tool's self.system binding is refused"
    );
}

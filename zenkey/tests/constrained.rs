//! `spec/scenarios/constrained.md` §2, §3 and §5 (spec §1.6, R7, §3.3), the
//! parts a Rust runtime can check. Both scenarios name a zenoh-pico owner; here a
//! session without a namespace stands in for it, and the pico itself is
//! spike S15's.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{T, client, client_ns, config, contract, eventually, imp, router};
use zenkey::ServiceBuilder;
use zenkey::consumer::{Liveness, Presence};
use zenkey::model::grammar::IfaceId;
use zenkey::model::template::Bindings;
use zenkey::presence::{self, Found};
use zenoh::sample::Sample;

/// §2: an owner without session namespaces writes the prefix literally,
/// and a session in namespace `dep1` reads its keys as base-relative.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_a_literal_prefix() {
    let (_r1, ep) = router(None).await;
    let literal = client(&ep).await;
    let reader = client_ns(&ep, Some("dep1")).await;
    let sub = reader
        .declare_subscriber("zk2/**")
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    let p = literal
        .declare_publisher("dep1/zk2/g/pico/probe.v1/stream/s")
        .await
        .unwrap();
    eventually("the reader is matched", || async {
        p.matching_status().await.unwrap().matching()
    })
    .await;
    p.put("v").await.unwrap();
    let s = tokio::time::timeout(common::SETTLE, sub.recv_async())
        .await
        .expect("a sample")
        .unwrap();
    assert_eq!(s.key_expr().as_str(), "zk2/g/pico/probe.v1/stream/s");
}

/// §5: the descriptor fits one fragment (4 KB, zenoh-pico's default
/// `Z_FRAG_MAX_SIZE`) and a GET of the instance key returns it whole.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_one_fragment_descriptor() {
    let (_r1, ep) = router(None).await;
    let owner = client(&ep).await;
    let tool = client(&ep).await;
    let probe: IfaceId = "probe.v1".parse().unwrap();
    let mut b = ServiceBuilder::new(&owner, config("c5/dev"));
    b.implement(imp("probe.v1")).unwrap();
    for res in [
        "stream/s",
        "@stream/xs",
        "state/st",
        "@state/xst",
        "events/ev",
        "@op/op",
    ] {
        b.expose(&probe, res).unwrap();
    }
    let svc = b.start().await.unwrap();
    let served = svc.descriptor_bytes();
    assert!(served.len() < 4096, "{} bytes", served.len());
    let key = svc.instance_key().unwrap().to_string();
    eventually("alive", || async {
        presence::liveliness_keys(&tool, &key, T).await.unwrap() == [key.clone()]
    })
    .await;
    let Found::Descriptor(_, got) = presence::descriptor(&tool, svc.address(), svc.instance(), T)
        .await
        .unwrap()
    else {
        panic!("no descriptor")
    };
    assert_eq!(&*got, &*served, "returned whole");
}

/// §3: across a constrained face the binding resolves without presence, and
/// while nothing crosses a tool reports the provider *unobservable*, never
/// down.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_static_binding_and_unobservable_liveness() {
    let (_r1, ep) = router(None).await;
    let vehicle = client(&ep).await;
    let ground = client(&ep).await;
    let det: IfaceId = "detections.v1".parse().unwrap();

    // The ground binds statically; presence across the face is denied.
    let mut b = ServiceBuilder::new(
        &ground,
        config("ground/tracker").bind("sources", &["vehicle-01/det"]),
    );
    b.require("sources", det.clone(), false);
    let ground_svc = b.start().await.unwrap();
    let consumer = ground_svc
        .consumer("sources", Arc::new(contract("detections.v1")))
        .unwrap()
        .with_presence(Presence::Unavailable);
    let sub = consumer.subscribe("stream/objects", |_| {}).await.unwrap();
    let provider = common::addr("vehicle-01/det");
    let window = Duration::from_millis(300);

    // Nothing has crossed: unobservable, not down.
    assert_eq!(
        consumer.liveness(&sub, &provider, window, T).await.unwrap(),
        Liveness::Unobservable
    );

    let mut b = ServiceBuilder::new(&vehicle, config("vehicle-01/det"));
    b.implement(imp("detections.v1")).unwrap();
    let w = b
        .declare_writer(&det, "stream/objects", &Bindings::new())
        .await
        .unwrap();
    let _veh = b.start().await.unwrap();
    eventually("a sample crossed", || async {
        w.put("x").await.unwrap();
        consumer.liveness(&sub, &provider, window, T).await.unwrap() == Liveness::Fresh
    })
    .await;
    // Silence again: back to unobservable.
    tokio::time::sleep(window * 2).await;
    assert_eq!(
        consumer.liveness(&sub, &provider, window, T).await.unwrap(),
        Liveness::Unobservable
    );
}

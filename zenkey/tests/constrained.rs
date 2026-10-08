//! `spec/scenarios/constrained.md` §2 and §5 (spec §1.6, §3.3), the parts
//! a Rust runtime can check. Both scenarios name a zenoh-pico owner; here a
//! session without a namespace stands in for it, and the pico itself is
//! spike S15's.

mod common;

use common::{T, client, client_ns, config, eventually, imp, router};
use zenkey::ServiceBuilder;
use zenkey::model::grammar::IfaceId;
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

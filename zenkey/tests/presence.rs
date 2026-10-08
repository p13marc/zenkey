//! `spec/scenarios/presence.md` §1–§5 (spec §1.5, §3.3, §8.1–§8.2).

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use common::{T, client, config, contract, eventually, imp, router};
use zenkey::model::descriptor::check;
use zenkey::model::grammar::{IfaceId, ZkKey, parse};
use zenkey::model::template::Bindings;
use zenkey::presence::{self, Found};
use zenkey::retrieval::{Retrieved, fetch_bundle};
use zenkey::{Error, Service, ServiceBuilder};
use zenoh::Wait;
use zenoh::query::Queryable;
use zenoh::sample::{Sample, SampleKind};

fn nav() -> IfaceId {
    "nav.v2".parse().unwrap()
}

/// A queryable callback replying `text` on `key`.
fn answer(key: zenoh::key_expr::OwnedKeyExpr, text: &'static str) -> impl Fn(zenoh::query::Query) {
    move |q| {
        let _ = q.reply(key.clone(), text).wait();
    }
}

/// A `nav.v2` owner at `address` holding `caps`: the required state and
/// operation answer, the member template is exposed, and `covariance` is
/// declared when `imu` is held. The queryables are returned to be kept.
async fn nav_builder(
    s: &zenoh::Session,
    address: &str,
    caps: &[&str],
) -> (ServiceBuilder, Vec<Queryable<()>>) {
    let mut cfg = config(address);
    for c in caps {
        cfg = cfg.capability(c);
    }
    nav_builder_with(s, cfg).await
}

async fn nav_builder_with(
    s: &zenoh::Session,
    cfg: zenkey::ServiceConfig,
) -> (ServiceBuilder, Vec<Queryable<()>>) {
    let imu = cfg.capabilities.contains("imu");
    let mut b = ServiceBuilder::new(s, cfg);
    b.implement(imp("nav.v2")).unwrap();
    let mut held = Vec::new();
    for (res, text) in [("state/pose", "here"), ("@op/goto", "ok")] {
        let key = b.key(&nav(), res, &Bindings::new()).unwrap().into_keyexpr();
        held.push(
            b.declare_queryable(&nav(), res, Some(&Bindings::new()), answer(key, text))
                .await
                .unwrap(),
        );
    }
    if imu {
        let key = b
            .key(&nav(), "state/covariance", &Bindings::new())
            .unwrap()
            .into_keyexpr();
        held.push(
            b.declare_queryable(
                &nav(),
                "state/covariance",
                Some(&Bindings::new()),
                answer(key, "0.1"),
            )
            .await
            .unwrap(),
        );
    }
    b.expose(&nav(), "state/tracks/{track}").unwrap();
    (b, held)
}

async fn keys(s: &zenoh::Session, sel: &str) -> Vec<String> {
    presence::liveliness_keys(s, sel, T).await.unwrap()
}

/// Waits until `svc`'s instance token is visible to `tool`: a tool acts on
/// presence (alive ⇒ callable, §8.2), not on the owner's `start` returning.
async fn seen(tool: &zenoh::Session, svc: &Service) {
    let key = svc.instance_key().unwrap().to_string();
    eventually("the instance token", || async {
        keys(tool, &key).await == [key.clone()]
    })
    .await;
}

/// §1: alive ⇒ callable; the descriptor and the bundle answer the moment
/// the instance token appears; owners hold an instance and an interface
/// token, a pure consumer an instance token only, and an interface exposing
/// nothing has no token.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_bring_up_order_and_tokens() {
    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let owner = client(&ep).await;
    let consumer = client(&ep).await;

    let sub = tool
        .liveliness()
        .declare_subscriber("zk2/p1/*/@zk/**")
        .history(true)
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();

    let starting = tokio::spawn(async move {
        let (mut b, held) = nav_builder(&owner, "p1/nav", &[]).await;
        // camera.v1's only resource is gated on a capability not held.
        b.implement(imp("camera.v1")).unwrap();
        let svc = b.start().await.unwrap();
        (svc, held, owner)
    });

    let goto = config("p1/nav");
    let goto_key = format!("zk2/{}/nav.v2/@op/goto", goto.address);
    let (mut called, mut described) = (false, false);
    while !(called && described) {
        let s = tokio::time::timeout(common::SETTLE, sub.recv_async())
            .await
            .expect("a token")
            .unwrap();
        if s.kind() != SampleKind::Put {
            continue;
        }
        match parse(s.key_expr().as_str()).unwrap() {
            ZkKey::Alive { iface, .. } if iface == nav() && !called => {
                let replies = tool.get(&goto_key).timeout(T).await.unwrap();
                let r = replies.recv_async().await.expect("the call is answered");
                let bytes = r.result().expect("an ok reply").payload().to_bytes();
                assert_eq!(&*bytes, b"ok", "alive ⇒ callable");
                called = true;
            }
            ZkKey::Instance { addr, instance } if !described => {
                let d = presence::descriptor(&tool, &addr, &instance, T)
                    .await
                    .unwrap()
                    .into_descriptor()
                    .expect("the descriptor answers at once");
                let fp = zenkey::model::canonical::Fingerprint::parse(&d.interfaces[0].contract)
                    .unwrap();
                let got = fetch_bundle(&tool, &nav(), &fp, T).await.unwrap();
                assert!(
                    matches!(got, Retrieved::Bundle(..)),
                    "the bundle answers at once"
                );
                described = true;
            }
            _ => {}
        }
    }
    let (svc, _held, _owner) = starting.await.unwrap();

    // A pure consumer: a role, bound by configuration, and nothing exposed.
    let mut b = ServiceBuilder::new(&consumer, config("p1/viewer").bind("nav", &["p1/nav"]));
    b.require("nav", nav(), false);
    let viewer = b.start().await.unwrap();

    let inst = svc.instance().to_string();
    eventually("both instances and the one interface token", || async {
        keys(&tool, "zk2/p1/*/@zk/**").await.len() == 3
    })
    .await;
    let all = keys(&tool, "zk2/p1/*/@zk/**").await;
    let fp16 = zenkey::Implementation::new(contract("nav.v2"))
        .fingerprint()
        .hex()
        .fp16();
    let want: BTreeSet<String> = [
        format!("zk2/p1/nav/@zk/instance/{inst}"),
        format!("zk2/p1/nav/@zk/alive/nav.v2/{inst}/{fp16}"),
        format!("zk2/p1/viewer/@zk/instance/{}", viewer.instance()),
    ]
    .into();
    assert_eq!(
        all.into_iter().collect::<BTreeSet<_>>(),
        want,
        "no camera.v1 token"
    );
    assert_eq!(svc.tokens_held(), [nav()]);
}

/// §2: the descriptor validates; losing a capability puts a new one in
/// which the gated resource's absence is implied, not listed; a required
/// resource missing keeps the owner down.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_the_descriptor() {
    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let owner = client(&ep).await;

    let (b, _held) = nav_builder(&owner, "p2/nav", &["imu"]).await;
    let mut svc = b.start().await.unwrap();
    seen(&tool, &svc).await;

    // 1. It validates, exposure included.
    let nav_c = contract("nav.v2");
    let Found::Descriptor(_, bytes) = presence::descriptor(&tool, svc.address(), svc.instance(), T)
        .await
        .unwrap()
    else {
        panic!("no descriptor")
    };
    let (d, report) = check(std::str::from_utf8(&bytes).unwrap(), &[&nav_c]);
    assert!(report.0.is_empty(), "{report}");
    assert_eq!(d.unwrap().capabilities, ["imu"]);

    // 2. The capability is lost.
    let puts = tool
        .declare_subscriber(svc.instance_key().unwrap().into_keyexpr())
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    // The subscriber must have reached the router before the put: a probe
    // publisher on the owner's side matching it is the proof.
    let probe = owner
        .declare_publisher(svc.instance_key().unwrap().into_keyexpr())
        .await
        .unwrap();
    eventually("the descriptor subscriber is known", || async {
        probe.matching_status().await.unwrap().matching()
    })
    .await;
    drop(probe);
    svc.set_capabilities(BTreeSet::new()).await.unwrap();
    let put = tokio::time::timeout(common::SETTLE, puts.recv_async())
        .await
        .expect("a new descriptor is put")
        .unwrap();
    let text = String::from_utf8(put.payload().to_bytes().into_owned()).unwrap();
    let (d, report) = check(&text, &[&nav_c]);
    assert!(
        report.0.is_empty(),
        "the implied absence is not listed (no D006): {report}"
    );
    let d = d.unwrap();
    assert!(d.capabilities.is_empty());
    assert!(d.interfaces[0].unavailable.is_empty());
    let again = presence::descriptor(&tool, svc.address(), svc.instance(), T)
        .await
        .unwrap()
        .into_descriptor()
        .unwrap();
    assert_eq!(again, d, "a GET returns the new descriptor");

    // 3. A required resource is missing: the owner does not start.
    let mut b = ServiceBuilder::new(&owner, config("p2/broken"));
    b.implement(imp("nav.v2")).unwrap();
    b.expose(&nav(), "@op/goto").unwrap();
    b.expose(&nav(), "state/tracks/{track}").unwrap();
    let err = b.start().await.err().expect("refused");
    assert!(
        matches!(err, Error::NotExposed(ref m) if m.contains("state/pose")),
        "{err}"
    );
    assert!(
        keys(&tool, "zk2/p2/broken/@zk/**").await.is_empty(),
        "no token appears"
    );
}

/// Tracks the live tokens under a selector from a liveliness subscriber,
/// asserting a bound on their count after every change once one was seen.
fn live_set(
    min: usize,
    max: usize,
) -> (
    Arc<Mutex<BTreeSet<String>>>,
    impl Fn(Sample) + Send + Sync + 'static,
) {
    let live = Arc::new(Mutex::new(BTreeSet::new()));
    let seen = Arc::new(Mutex::new(false));
    let l = Arc::clone(&live);
    let cb = move |s: Sample| {
        let mut l = l.lock().unwrap();
        match s.kind() {
            SampleKind::Put => l.insert(s.key_expr().as_str().to_owned()),
            SampleKind::Delete => l.remove(s.key_expr().as_str()),
        };
        let mut seen = seen.lock().unwrap();
        *seen |= !l.is_empty();
        if *seen {
            assert!((min..=max).contains(&l.len()), "{} live: {l:?}", l.len());
        }
    };
    (live, cb)
}

/// §3: re-minting is make-before-break, and a member's token cycles alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_epochs_and_re_minting() {
    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let owner = client(&ep).await;

    let (live, cb) = live_set(1, 2);
    let _watch = tool
        .liveliness()
        .declare_subscriber("zk2/p3/nav/@zk/instance/*")
        .history(true)
        .callback(cb)
        .await
        .unwrap();
    let (b, _held) = nav_builder(&owner, "p3/nav", &[]).await;
    let mut svc = b.start().await.unwrap();
    let first = svc.instance().clone();
    eventually("the first instance token", || async {
        live.lock().unwrap().len() == 1
    })
    .await;

    // 1. Counters reset: three re-mints.
    for _ in 0..3 {
        svc.new_epoch().await.unwrap();
    }
    let now = format!("zk2/p3/nav/@zk/instance/{}", svc.instance());
    eventually("only the newest instance token", || async {
        *live.lock().unwrap() == BTreeSet::from([now.clone()])
    })
    .await;
    assert_ne!(*svc.instance(), first);
    let alive = keys(&tool, "zk2/p3/nav/@zk/alive/**").await;
    assert_eq!(alive.len(), 1);
    assert!(
        alive[0].contains(svc.instance().as_str()),
        "the interface token moved too"
    );
    let d = presence::descriptor(&tool, svc.address(), svc.instance(), T)
        .await
        .unwrap()
        .into_descriptor()
        .unwrap();
    assert_eq!(d.instance, svc.instance().as_str());

    // 2. Members: `a` loses continuity, `b` is untouched.
    let events: Arc<Mutex<Vec<(SampleKind, String)>>> = Arc::default();
    let e = Arc::clone(&events);
    let _members = tool
        .liveliness()
        .declare_subscriber("zk2/p3/nav/@zk/member/**")
        .history(true)
        .callback(move |s| {
            e.lock()
                .unwrap()
                .push((s.kind(), s.key_expr().as_str().to_owned()))
        })
        .await
        .unwrap();
    let a1 = svc.declare_member(&nav(), "a").await.unwrap();
    let b1 = svc.declare_member(&nav(), "b").await.unwrap();
    eventually("two members", || async {
        events.lock().unwrap().len() == 2
    })
    .await;
    let a2 = svc.cycle_member(&nav(), "a").await.unwrap();
    assert_ne!(a1, a2);
    eventually("a's token cycled", || async {
        events.lock().unwrap().len() == 4
    })
    .await;
    let ev = events.lock().unwrap().clone();
    let k = |m: &str, e: &zenkey::model::grammar::InstanceId| {
        format!("zk2/p3/nav/@zk/member/nav.v2/{m}/{e}")
    };
    assert_eq!(
        ev[2..],
        [
            (SampleKind::Put, k("a", &a2)),
            (SampleKind::Delete, k("a", &a1))
        ],
        "new first, then old"
    );
    assert!(
        !ev.iter()
            .any(|(kind, key)| *kind == SampleKind::Delete && *key == k("b", &b1))
    );
    svc.close().await.unwrap();
}

/// §5: with `health.v1` in the tokenless set, 100 services hold 200 tokens,
/// and a tool still finds every `health.v1` provider.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_tokenless_set() {
    const N: usize = 100;
    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let host = client(&ep).await;
    let health: IfaceId = "health.v1".parse().unwrap();
    let mut services: Vec<(Service, Vec<Queryable<()>>)> = Vec::new();
    for i in 0..N {
        let address = format!("p5/dev{i}");
        // The deployment's tokenless set.
        let cfg = config(&address).tokenless(health.clone());
        let (mut b, mut held) = nav_builder_with(&host, cfg).await;
        b.implement(imp("health.v1")).unwrap();
        let key = b
            .key(&health, "state/status", &Bindings::new())
            .unwrap()
            .into_keyexpr();
        held.push(
            b.declare_queryable(
                &health,
                "state/status",
                Some(&Bindings::new()),
                answer(key, "ok"),
            )
            .await
            .unwrap(),
        );
        services.push((b.start().await.unwrap(), held));
    }
    eventually("200 tokens", || async {
        keys(&tool, "zk2/p5/*/@zk/**").await.len() == 2 * N
    })
    .await;
    let all = keys(&tool, "zk2/p5/*/@zk/**").await;
    let parsed: Vec<ZkKey> = all.iter().map(|k| parse(k).unwrap()).collect();
    let count = |f: &dyn Fn(&ZkKey) -> bool| parsed.iter().filter(|k| f(k)).count();
    assert_eq!(count(&|k| matches!(k, ZkKey::Instance { .. })), N);
    assert_eq!(
        count(&|k| matches!(k, ZkKey::Alive { iface, .. } if *iface == nav())),
        N
    );
    assert_eq!(
        count(&|k| matches!(k, ZkKey::Alive { iface, .. } if *iface == health)),
        0
    );

    // The tool finds the health.v1 providers through instance tokens and
    // descriptors.
    let mut providers = BTreeMap::new();
    for k in &parsed {
        if let ZkKey::Instance { addr, instance } = k {
            let d = presence::descriptor(&tool, addr, instance, T)
                .await
                .unwrap()
                .into_descriptor()
                .unwrap();
            let e = d
                .interfaces
                .iter()
                .find(|e| e.iface == "health.v1")
                .expect("listed");
            assert!(!e.token, "marked tokenless");
            providers.insert(addr.to_string(), e.token);
        }
    }
    assert_eq!(providers.len(), N);
}

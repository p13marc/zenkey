//! `spec/scenarios/presence.md` §1–§5 (spec §1.5, §3.3, §8.1–§8.2).

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use common::{T, client, config, contract, eventually, example, imp, router};
use zenkey::model::descriptor::{Cause, check};
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

/// §1: alive ⇒ callable, and the state the owner started with is there;
/// the descriptor and the bundle answer the moment the instance token
/// appears, and a subscriber up before the start receives the first
/// descriptor without a GET; the templated state with no member is exposed
/// by its template, unlisted, with no member token; owners hold an instance
/// and an interface token, a pure consumer an instance token only, and an
/// interface exposing nothing has no token.
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
    // A data subscriber to instance keys, up before the owner starts.
    let descriptors = tool
        .declare_subscriber("zk2/*/*/@zk/instance/*")
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    let probe = owner
        .declare_publisher("zk2/p1/nav/@zk/instance/ffffffffffffffff")
        .await
        .unwrap();
    eventually("the descriptor subscriber is known", || async {
        probe.matching_status().await.unwrap().matching()
    })
    .await;
    drop(probe);

    let starting = tokio::spawn(async move {
        let mut b = ServiceBuilder::new(&owner, config("p1/nav"));
        b.implement(imp("nav.v2")).unwrap();
        // The value it holds at start, put before the tokens (core §8.2,
        // "State values", F-68).
        let none = Bindings::new();
        let pose = b
            .declare_state_writer(&nav(), "state/pose", &none)
            .await
            .unwrap();
        pose.put("here").await.unwrap();
        let key = b.key(&nav(), "@op/goto", &none).unwrap().into_keyexpr();
        let goto = b
            .declare_queryable(&nav(), "@op/goto", Some(&none), answer(key, "ok"))
            .await
            .unwrap();
        // The templated state, exposed by its template with no member.
        b.expose(&nav(), "state/tracks/{track}").unwrap();
        // camera.v1's only resource is gated on a capability not held.
        b.implement(imp("camera.v1")).unwrap();
        let svc = b.start().await.unwrap();
        (svc, (pose, goto), owner)
    });

    let goto = config("p1/nav");
    let goto_key = format!("zk2/{}/nav.v2/@op/goto", goto.address);
    let pose_key = format!("zk2/{}/nav.v2/state/pose", goto.address);
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
                let state = tool
                    .get(&pose_key)
                    .target(zenoh::query::QueryTarget::All)
                    .consolidation(zenoh::query::ConsolidationMode::Latest)
                    .timeout(T)
                    .await
                    .unwrap();
                let r = state.recv_async().await.expect("the state is answered");
                let v = r.result().expect("the value it started with");
                assert_eq!(&*v.payload().to_bytes(), b"here");
                assert!(v.timestamp().is_some(), "stamped (S2)");
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

    // The first descriptor, put on the instance key (core §3.3, §8.2 step 3).
    let put = tokio::time::timeout(common::SETTLE, descriptors.recv_async())
        .await
        .expect("the first descriptor is put")
        .unwrap();
    assert_eq!(
        put.key_expr().as_str(),
        svc.instance_key().unwrap().as_str()
    );
    let (d, report) = check(
        std::str::from_utf8(&put.payload().to_bytes()).unwrap(),
        &[&contract("nav.v2")],
    );
    assert!(report.0.is_empty(), "{report}");
    let d = d.unwrap();
    assert_eq!(d.instance, svc.instance().as_str());
    assert!(
        d.interfaces.iter().all(|e| e.unavailable.is_empty()),
        "the template with no member is exposed, not listed"
    );

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
        "no camera.v1 token, and no member token"
    );
    assert_eq!(svc.tokens_held(), [nav()]);
}

/// §2: the descriptor validates; losing a capability puts a new one in
/// which the gated resource's absence is implied, not listed; each of step
/// 2's refusals keeps the owner down.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_the_descriptor() {
    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let owner = client(&ep).await;

    let (mut b, _held) = nav_builder(&owner, "p2/nav", &["imu"]).await;
    // An interface that uses a profile, and an optional role left unbound.
    let plan: IfaceId = "mission_plan.v1".parse().unwrap();
    b.implement(zenkey::Implementation::new(example(
        "walkthrough/mission_plan.v1",
    )))
    .unwrap();
    b.expose(&plan, "state/plans/{vehicle}").unwrap();
    b.expose(&plan, "@op/list").unwrap();
    b.require("telemetry", "detections.v1".parse().unwrap(), true);
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
    let plan_c = example("walkthrough/mission_plan.v1");
    let (d, report) = check(std::str::from_utf8(&bytes).unwrap(), &[&nav_c, &plan_c]);
    assert!(report.0.is_empty(), "{report}");
    let d = d.unwrap();
    assert_eq!(d.capabilities, ["imu"]);
    assert_eq!(
        d.profiles,
        ["desired.v1"],
        "the union of the contracts' uses"
    );
    let tele = d
        .requires
        .iter()
        .find(|r| r.role == "telemetry")
        .expect("listed");
    assert!(
        tele.bindings.is_empty() && tele.declared_by.is_none(),
        "unbound optional: bindings []"
    );
    // One reply, application/json, with consolidation None.
    let rx = tool
        .get(svc.instance_key().unwrap().into_keyexpr())
        .consolidation(zenoh::query::ConsolidationMode::None)
        .timeout(T)
        .with(flume::unbounded::<zenoh::query::Reply>())
        .await
        .unwrap();
    let mut replies = Vec::new();
    while let Ok(r) = rx.recv_async().await {
        replies.push(r.into_result().expect("an ok reply"));
    }
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].encoding().to_string(), "application/json");

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
    let (d, report) = check(&text, &[&nav_c, &plan_c]);
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

    // Steps 3 to 5, each watched through R1 and against its control.
    // 3. A required resource is missing: the owner does not start.
    let broken = |pose: bool| {
        let mut b = ServiceBuilder::new(&owner, config("p2/broken"));
        b.implement(imp("nav.v2")).unwrap();
        b.expose(&nav(), "@op/goto").unwrap();
        b.expose(&nav(), "state/tracks/{track}").unwrap();
        if pose {
            b.expose(&nav(), "state/pose").unwrap();
        }
        b
    };
    refused_while_watched(&tool, "p2/broken", broken(false), "state/pose").await;
    controlled(&tool, "p2/broken", broken(true)).await;

    // 4. A required role the configuration binds to nothing: no start
    //    (core §3.2).
    let unbound = |bound: bool| {
        let mut cfg = config("p2/unbound");
        if bound {
            cfg = cfg.bind("cmd", &["p2/teleop"]);
        }
        let mut b = ServiceBuilder::new(&owner, cfg);
        b.require("cmd", "twist_cmd.v1".parse().unwrap(), false);
        b
    };
    refused_while_watched(&tool, "p2/unbound", unbound(false), "cmd").await;
    controlled(&tool, "p2/unbound", unbound(true)).await;

    // 5. An optional resource neither exposed nor listed `unavailable`, its
    //    gate's capability held: no start (core §8.2 step 2, F-70), since
    //    its descriptor would claim it.
    let gated = |listed: bool| {
        let mut b = ServiceBuilder::new(&owner, config("p2/gated").capability("imu"));
        b.implement(imp("nav.v2")).unwrap();
        for res in ["state/pose", "@op/goto", "state/tracks/{track}"] {
            b.expose(&nav(), res).unwrap();
        }
        if listed {
            b.unavailable(&nav(), "state/covariance", Cause::Config, None)
                .unwrap();
        }
        b
    };
    refused_while_watched(&tool, "p2/gated", gated(false), "state/covariance").await;
    controlled(&tool, "p2/gated", gated(true)).await;

    // 6. `archive.v1` in the tokenless set: no start, whether or not the
    //    owner implements it (core §4.4, §8.2 step 2, 0.17).
    let tokenless = |named: bool| {
        let mut cfg = config("p2/tokenless");
        if named {
            cfg = cfg.tokenless("archive.v1".parse().unwrap());
        }
        let mut b = ServiceBuilder::new(&owner, cfg);
        b.implement(imp("nav.v2")).unwrap();
        for res in ["state/pose", "@op/goto", "state/tracks/{track}"] {
            b.expose(&nav(), res).unwrap();
        }
        b
    };
    refused_while_watched(&tool, "p2/tokenless", tokenless(true), "archive.v1").await;
    controlled(&tool, "p2/tokenless", tokenless(false)).await;
}

/// A liveliness subscriber to `address`'s tokens through R1, up and known
/// before an owner is launched (presence.md §2, "Watching a refusal").
async fn watch(
    tool: &zenoh::Session,
    address: &str,
) -> zenoh::pubsub::Subscriber<flume::Receiver<Sample>> {
    tool.liveliness()
        .declare_subscriber(format!("zk2/{address}/@zk/**"))
        .history(true)
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap()
}

/// Starts `b`, which refuses for `why`: the subscriber up before the launch
/// receives no token of the service for the wait of core §8.1 after the
/// refusal, and a liveliness GET afterwards finds none.
async fn refused_while_watched(tool: &zenoh::Session, address: &str, b: ServiceBuilder, why: &str) {
    let seen = watch(tool, address).await;
    let err = b.start().await.err().expect("refused");
    assert!(
        matches!(err, Error::NotExposed(ref m) if m.contains(why)),
        "{err}"
    );
    let token = tokio::time::timeout(T, seen.recv_async()).await;
    assert!(token.is_err(), "no token while watched: {token:?}");
    assert!(
        keys(tool, &format!("zk2/{address}/@zk/**"))
            .await
            .is_empty(),
        "no token afterwards"
    );
}

/// The control: the same owner, its refusal fixed, launched the same way,
/// shows its instance token to the same watch within the wait.
async fn controlled(tool: &zenoh::Session, address: &str, b: ServiceBuilder) {
    let seen = watch(tool, address).await;
    let svc = b.start().await.expect("the control starts");
    let instance = svc.instance_key().unwrap().to_string();
    let deadline = tokio::time::Instant::now() + common::SETTLE;
    loop {
        let s = tokio::time::timeout_at(deadline, seen.recv_async())
            .await
            .expect("the control's instance token")
            .unwrap();
        if s.key_expr().as_str() == instance {
            break;
        }
    }
    svc.close().await.unwrap();
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

    // 3. An `epoch` template, and no member declared yet: no member token
    //    (core §8.1, a member exists from its first declaration).
    assert!(keys(&tool, "zk2/p3/nav/@zk/member/**").await.is_empty());

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
/// and a tool still finds every `health.v1` provider. `health.v1` is the
/// standard contract, implemented by the runtime (#721); its verdicts are
/// `profile_health.rs` §5's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_tokenless_set() {
    const N: usize = 100;
    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let host = client(&ep).await;
    let health = zenkey::health::iface();
    let mut services: Vec<(Service, Vec<Queryable<()>>)> = Vec::new();
    for i in 0..N {
        let address = format!("p5/dev{i}");
        // The deployment's tokenless set.
        let cfg = config(&address).tokenless(health.clone());
        let (mut b, held) = nav_builder_with(&host, cfg).await;
        b.health()
            .await
            .unwrap()
            .set_status(zenkey::health::Level::Ok, "serving")
            .await
            .unwrap();
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

/// A TCP proxy to `upstream` whose downstream direction a test can stall:
/// bytes from the router are held, in order, until it is released, while the
/// client's still flow. The client's session stays up, and hears nothing.
async fn stalling_proxy(upstream: &str) -> (String, Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let target = upstream.trim_start_matches("tcp/").to_owned();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("tcp/{}", listener.local_addr().unwrap());
    let stalled = Arc::new(AtomicBool::new(false));
    let s = Arc::clone(&stalled);
    tokio::spawn(async move {
        while let Ok((down, _)) = listener.accept().await {
            let Ok(up) = tokio::net::TcpStream::connect(&target).await else {
                continue;
            };
            let ((mut dr, mut dw), (mut ur, mut uw)) = (down.into_split(), up.into_split());
            tokio::spawn(async move {
                let _ = tokio::io::copy(&mut dr, &mut uw).await;
            });
            let s = Arc::clone(&s);
            tokio::spawn(async move {
                let mut buf = vec![0; 1 << 16];
                while let Ok(n @ 1..) = ur.read(&mut buf).await {
                    while s.load(Ordering::SeqCst) {
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    }
                    if dw.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    (endpoint, stalled)
}

/// Core §8.1: a liveliness GET that ends at its timeout is read as possibly
/// incomplete, and its error reply is reported, not dropped (#660). The
/// router's replies are held back, so its final reply never comes in time.
/// The selector names the instance tokens: an ambient one (`zk2/**`)
/// selects no control token (§1.3), and its "no token" could never fail
/// (F-75, #670).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_read_that_ends_at_its_timeout_says_so() {
    use std::sync::atomic::Ordering;
    let instances = "zk2/*/*/@zk/instance/*";
    let (_r1, ep) = router(None).await;
    let (proxied, stalled) = stalling_proxy(&ep).await;
    let owner = client(&ep).await;
    let tool = client(&proxied).await;
    let (b, _held) = nav_builder(&owner, "p1/nav", &[]).await;
    let svc = b.start().await.unwrap();
    seen(&tool, &svc).await;
    let open = presence::liveliness_read(&tool, instances, T)
        .await
        .unwrap();
    assert!(open.complete && open.errors.is_empty(), "{open:?}");
    assert_eq!(open.keys, [svc.instance_key().unwrap().to_string()]);
    stalled.store(true, Ordering::SeqCst);
    let held = presence::liveliness_read(&tool, instances, std::time::Duration::from_millis(300))
        .await
        .unwrap();
    stalled.store(false, Ordering::SeqCst);
    assert!(!held.complete, "{held:?}");
    assert_eq!(held.errors, ["zenoh/string: Timeout"]);
    assert!(held.keys.is_empty(), "possibly incomplete, never absence");
}

/// Core §8.1 (0.8): a liveliness GET the router's access control refuses is
/// answered like one that matched nothing, with no error reply. The read is
/// complete and empty, the same as an absent provider's. The controls: the
/// router's own read of the same selector (no face, so no rule) holds the
/// token, and the tool's read of the instance tokens, which the rule leaves
/// alone, is answered. The rule acts on the query's ingress at the router;
/// on `egress` alone, measured, it refuses nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_read_looks_complete_and_empty() {
    let acl = r#"{
        enabled: true,
        default_permission: "allow",
        rules: [{
            id: "no-alive-reads",
            messages: ["liveliness_query"],
            flows: ["ingress"],
            permission: "deny",
            key_exprs: ["zk2/*/*/@zk/alive/**"],
        }],
        subjects: [{ id: "anyone" }],
        policies: [{ rules: ["no-alive-reads"], subjects: ["anyone"] }],
    }"#;
    let alive = "zk2/*/*/@zk/alive/**";
    let (r1, ep) = common::router_with(None, &[("access_control", acl)]).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let (b, _held) = nav_builder(&owner, "p1/nav", &[]).await;
    let svc = b.start().await.unwrap();
    eventually("the router holds the interface token", || async {
        keys(&r1, alive).await.len() == 1
    })
    .await;

    let instance = presence::liveliness_read(&tool, "zk2/*/*/@zk/instance/*", T)
        .await
        .unwrap();
    assert!(instance.complete, "{instance:?}");
    assert_eq!(instance.keys, [svc.instance_key().unwrap().to_string()]);
    let refused = presence::liveliness_read(&tool, alive, T).await.unwrap();
    assert!(refused.complete, "answered, not timed out: {refused:?}");
    assert!(refused.errors.is_empty(), "no error reply: {refused:?}");
    assert!(refused.keys.is_empty(), "{refused:?}");
}

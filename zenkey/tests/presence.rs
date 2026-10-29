//! `spec/scenarios/presence.md` §1–§9 (spec §1.5, §3.3, §8.1–§8.2; §7–§9,
//! what a complete presence read is, core 0.37 and 0.38). §8's and §9's
//! reader under test is `zenkey-fleet`'s `PresenceFeed`, in that crate's
//! `tests/presence_scenarios.rs`: this crate cannot depend on it, so its
//! `s8_*` and `s9_*` observe zenoh and the runtime's own reads.

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
    // The read the poll found, never a second one: under load a second
    // read can come back without a token the first saw.
    let all = common::eventually_some("both instances and the one interface token", || async {
        let all = keys(&tool, "zk2/p1/*/@zk/**").await;
        (all.len() == 3).then_some(all)
    })
    .await;
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

/// The raw liveliness GET of zenoh's own API, as §7 and §9 need it: whether
/// it ended with no error reply, and the keys it returned, once each.
async fn raw_read(s: &zenoh::Session, sel: &str) -> (bool, Vec<String>) {
    let replies = s.liveliness().get(sel).timeout(T).await.unwrap();
    let (mut complete, mut keys) = (true, Vec::new());
    while let Ok(r) = replies.recv_async().await {
        match r.result() {
            Ok(sample) => keys.push(sample.key_expr().as_str().to_owned()),
            Err(_) => complete = false,
        }
    }
    keys.sort();
    keys.dedup();
    (complete, keys)
}

/// §7 (core §8.1, 0.37): a read through a session cut off from every other
/// zenoh node is answered at once, complete and empty, though the owner
/// never left; the reference makes no such read, and says why.
///
/// Step 5 (0.38, F-185): a read asked while the link is stalled, its
/// transport still held, and cut while it is in flight, is answered at the
/// cut with the final reply alone. The check before it passed; the one
/// after it finds no link, and the read is possibly incomplete, cut off
/// during it, never the owner absent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_a_read_through_a_session_cut_off() {
    let (r1, ep) = router(None).await;
    let owner = client(&ep).await;
    let link = common::link_to(&ep).await;
    let tool = common::retrying_client(link.endpoint()).await;
    let (b, _held) = nav_builder(&owner, "p7/nav", &[]).await;
    let svc = b.start().await.unwrap();
    seen(&tool, &svc).await;
    let key = svc.instance_key().unwrap().to_string();

    link.cut();
    eventually("the tool holds no transport", || async {
        presence::cut_off(&tool).await
    })
    .await;
    let started = std::time::Instant::now();
    assert_eq!(
        raw_read(&tool, "zk2/*/*/@zk/**").await,
        (true, Vec::new()),
        "complete and empty"
    );
    assert!(started.elapsed() < T, "answered at once, nothing asked");
    assert!(
        keys(&r1, "zk2/*/*/@zk/instance/*").await.contains(&key),
        "the owner never left"
    );
    let refused = presence::liveliness_read(&tool, "zk2/*/*/@zk/**", T)
        .await
        .expect_err("not made");
    assert!(matches!(refused, Error::CutOff { .. }), "{refused}");

    link.heal();
    eventually("the tool reads the owner again", || async {
        presence::liveliness_read(&tool, "zk2/*/*/@zk/instance/*", T)
            .await
            .is_ok_and(|r| r.complete && r.keys.contains(&key))
    })
    .await;

    // 5. A read in flight when the link is cut (0.38, F-185). Its timeout
    // is long, so an answer at the cut cannot be the timeout's.
    let long = std::time::Duration::from_secs(10);
    link.stall();
    assert!(!presence::cut_off(&tool).await, "stalled, the link is held");
    let reader = tool.clone();
    let asked =
        tokio::spawn(
            async move { presence::liveliness_read(&reader, "zk2/*/*/@zk/**", long).await },
        );
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(!asked.is_finished(), "held by the stall, in flight");
    let cut_at = std::time::Instant::now();
    link.cut();
    let read = asked
        .await
        .unwrap()
        .expect("made: the check before it passed");
    let answered = cut_at.elapsed();
    assert!(
        answered < long / 2,
        "answered at the cut, not at its timeout: {answered:?}"
    );
    assert!(
        read.errors.is_empty() && read.keys.is_empty(),
        "the final reply alone, no token and no error reply: {read:?}"
    );
    assert!(
        read.cut_off_during && !read.complete,
        "possibly incomplete, cut off during it: {read:?}"
    );
    assert_eq!(read.incomplete_reason(), Some(presence::CUT_OFF_DURING));
    assert!(
        keys(&r1, "zk2/*/*/@zk/instance/*").await.contains(&key),
        "the owner never left"
    );
    link.heal();
}

/// §8 (core §8.1, 0.37): a withdrawal heard live is not absence. The
/// tool's link cut, its watch session's subscriber hears the owner's token
/// withdrawn though the owner never left, and hears it again on the heal;
/// the owner stopping is withdrawn too, and the read after it, complete and
/// without the token, is what establishes absence.
///
/// This is zenoh's half of §8. Its Expected observes the reader under test
/// (0.38, F-188), which is the fleet's `PresenceFeed`: that half is
/// `zenkey-fleet/tests/presence_scenarios.rs`'s
/// `s8_a_withdrawal_heard_live`, since this crate cannot depend on the
/// fleet.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s8_a_withdrawal_heard_live() {
    let (r1, ep) = router(None).await;
    let owner = client(&ep).await;
    let link = common::link_to(&ep).await;
    let tool = common::retrying_client(link.endpoint()).await;
    let watch = common::retrying_client(link.endpoint()).await;
    let heard: Arc<Mutex<Vec<(String, bool)>>> = Arc::default();
    let h = Arc::clone(&heard);
    let _sub = presence::declare_subscriber(&watch, "zk2/*/*/@zk/instance/*")
        .history(true)
        .callback(move |s: Sample| {
            h.lock().unwrap().push((
                s.key_expr().as_str().to_owned(),
                s.kind() == SampleKind::Put,
            ));
        })
        .await
        .unwrap();
    let (b, _held) = nav_builder(&owner, "p8/nav", &[]).await;
    let svc = b.start().await.unwrap();
    let key = svc.instance_key().unwrap().to_string();
    let last = |heard: &Arc<Mutex<Vec<(String, bool)>>>| {
        heard
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(k, _)| *k == key)
            .map(|(_, put)| *put)
    };
    eventually("the subscriber hears the owner", || async {
        last(&heard) == Some(true)
    })
    .await;
    seen(&tool, &svc).await;

    link.cut();
    eventually("the withdrawal is heard", || async {
        last(&heard) == Some(false)
    })
    .await;
    assert!(
        keys(&r1, "zk2/*/*/@zk/instance/*").await.contains(&key),
        "heard withdrawn, and the owner never left"
    );
    link.heal();
    eventually("heard again", || async { last(&heard) == Some(true) }).await;
    eventually("and read", || async {
        presence::liveliness_read(&tool, &key, T)
            .await
            .is_ok_and(|r| r.complete && r.keys == [key.clone()])
    })
    .await;

    svc.close().await.unwrap();
    eventually("the stop is heard", || async {
        last(&heard) == Some(false)
    })
    .await;
    eventually("absence, as a read establishes it", || async {
        presence::liveliness_read(&tool, "zk2/*/*/@zk/instance/*", T)
            .await
            .is_ok_and(|r| r.complete && !r.keys.contains(&key))
    })
    .await;
}

/// §9 step 2's runs, each on a fresh session (0.42, F-235).
const S9_RUNS: usize = 10;

/// §9 (core §8.1, 0.37; Appendix B; #830): a liveliness GET made at once
/// beside a history liveliness subscriber of the same session completes
/// without the owner's live token, nearly always (0.42, F-235: the
/// session's own answer can win the race, a few runs in a hundred), so
/// step 2 runs on [`S9_RUNS`] fresh sessions and asserts a miss in one at
/// least; the same GET on a session that holds no liveliness subscriber
/// holds it. The reference refuses a read on a session that holds a
/// subscriber it declared, and its barrier there returns.
///
/// Steps 1 to 3. Steps 4 and 5 observe the reader under test (0.38,
/// F-189), the fleet's `PresenceFeed`: `zenkey-fleet/tests/presence_scenarios.rs`'s
/// `s9_a_read_beside_a_liveliness_subscriber`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s9_a_read_beside_a_liveliness_subscriber() {
    let (_r1, ep) = router(None).await;
    let owner = client(&ep).await;
    let tool = client(&ep).await;
    let (b, _held) = nav_builder(&owner, "p9/nav", &[]).await;
    let svc = b.start().await.unwrap();
    seen(&tool, &svc).await;
    let key = svc.instance_key().unwrap().to_string();

    // zenoh's own API: the subscriber and the GET on one session, each run
    // on a fresh one. The miss is a race the session's own answer can win
    // (0.42, F-235), so one run proves nothing either way.
    let mut missed = 0;
    for _ in 0..S9_RUNS {
        let beside = client(&ep).await;
        let heard = Arc::new(Mutex::new(false));
        let h = Arc::clone(&heard);
        let k = key.clone();
        let _sub = beside
            .liveliness()
            .declare_subscriber("zk2/*/*/@zk/instance/*")
            .history(true)
            .callback(move |s: Sample| {
                if s.key_expr().as_str() == k {
                    *h.lock().unwrap() = true;
                }
            })
            .await
            .unwrap();
        let (complete, found) = raw_read(&beside, "zk2/*/*/@zk/**").await;
        assert!(complete, "the final reply, no error reply");
        missed += usize::from(!found.contains(&key));
        eventually("the subscriber hears it", || async {
            *heard.lock().unwrap()
        })
        .await;
        let _ = beside.close().await;
    }
    eprintln!("§9 step 2: the GET beside the subscriber missed the token in {missed} of {S9_RUNS}");
    assert!(
        missed > 0,
        "zenoh fixed #830's mechanism: revisit core §8.1's rule (0.37) and Appendix B — a GET \
         right after a history subscriber on the same session held the live token in each of \
         {S9_RUNS} runs"
    );
    assert!(raw_read(&tool, "zk2/*/*/@zk/**").await.1.contains(&key));

    // The reference: a session that holds a subscriber it declared makes no
    // read; its barrier returns; the tool's session reads.
    let watch = client(&ep).await;
    let _watched = presence::declare_subscriber(&watch, "zk2/*/*/@zk/instance/*")
        .history(true)
        .callback(|_| {})
        .await
        .unwrap();
    assert!(presence::watching(&watch) && !presence::watching(&tool));
    let refused = presence::liveliness_read(&watch, "zk2/*/*/@zk/**", T)
        .await
        .expect_err("refused");
    assert!(
        matches!(refused, Error::ReadBesideSubscriber { .. }),
        "{refused}"
    );
    assert!(
        presence::settle(&watch, "zk2/*/*/@zk/instance/*", T)
            .await
            .unwrap()
    );
    let read = presence::liveliness_read(&tool, "zk2/*/*/@zk/**", T)
        .await
        .unwrap();
    assert!(read.complete && read.keys.contains(&key), "{read:?}");
}

/// Core §8.1's implementation note (0.40; `redundancy.v1` text 0.2,
/// F-204), measured: a session's liveliness read returns the session's own
/// tokens even when its router's access control refuses the read, since
/// the session answers its own tokens itself. On another session the same
/// read through the same router is complete and empty (0.8's refusal), and
/// a read on the token's own session says nothing about the bus for that
/// token.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_read_on_a_tokens_own_session_still_returns_it() {
    let acl = r#"{
        enabled: true,
        default_permission: "allow",
        rules: [{
            id: "no-presence-reads",
            messages: ["liveliness_query"],
            flows: ["ingress"],
            permission: "deny",
            key_exprs: ["zk2/*/*/@zk/**"],
        }],
        subjects: [{ id: "anyone" }],
        policies: [{ rules: ["no-presence-reads"], subjects: ["anyone"] }],
    }"#;
    let (r1, ep) = common::router_with(None, &[("access_control", acl)]).await;
    let (owner, other) = (client(&ep).await, client(&ep).await);
    let key = "zk2/lab/ledger/@zk/instance/000000000000000a";
    let _token = owner.liveliness().declare_token(key).await.unwrap();
    eventually("the router holds the token", || async {
        keys(&r1, "zk2/lab/ledger/@zk/**").await == [key]
    })
    .await;
    let sel = "zk2/lab/ledger/@zk/**";
    for _ in 0..5 {
        let own = presence::liveliness_read(&owner, sel, T).await.unwrap();
        assert!(own.complete, "{own:?}");
        assert_eq!(own.keys, [key], "its own token, from the session itself");
        let refused = presence::liveliness_read(&other, sel, T).await.unwrap();
        assert!(refused.complete && refused.errors.is_empty(), "{refused:?}");
        assert!(
            refused.keys.is_empty(),
            "refused at the router: {refused:?}"
        );
    }
}

/// Core §8.1 (0.40; `redundancy.v1` text 0.2, F-206), measured against
/// zenoh 1.10.1 (`hat/peer/token.rs`: a router's south peer region, the
/// `auto` gateway's, propagates no token from one of its peers to another,
/// which are expected to be linked to each other): two peers behind one
/// router, neither listening, so neither can link to the other, one
/// holding a token. The other's read through the router is complete and
/// empty, every time, while a client of the same router reads the token.
/// `routing.router.peers_failover_brokering` has no effect in 1.10.1
/// (`zenoh-config`: deprecated, read and ignored). With both peers
/// listening and gossiping through a router that gossips, zenoh's default,
/// they link, and the token is read. A read is complete only for what its
/// routers' clients and its linked peers hold.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_peers_read_through_a_router_misses_an_unlinked_peers_token() {
    // The router gossips, zenoh's default, so that peers that listen learn
    // of each other through it.
    let (_r1, ep) = common::router_with(None, &[("scouting/gossip/enabled", "true")]).await;
    let tool = client(&ep).await;
    let sel = "zk2/lab/ledger/@zk/**";
    let key = "zk2/lab/ledger/@zk/instance/000000000000000a";
    let read = |s: &zenoh::Session| {
        let s = s.clone();
        async move { presence::liveliness_read(&s, sel, T).await.unwrap() }
    };
    // Neither listening, gossip on (zenoh's default) or off.
    for gossip in [true, false] {
        let holder = common::peer(&ep, false, gossip).await;
        let reader = common::peer(&ep, false, gossip).await;
        let token = holder.liveliness().declare_token(key).await.unwrap();
        eventually("a client of the router reads it", || async {
            let r = read(&tool).await;
            r.complete && r.keys == [key]
        })
        .await;
        for _ in 0..6 {
            let r = read(&reader).await;
            assert!(r.complete, "gossip {gossip}: {r:?}");
            assert!(
                r.keys.is_empty(),
                "gossip {gossip}: the peer missed it: {r:?}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        drop(token);
        for s in [holder, reader] {
            s.close().await.unwrap();
        }
        eventually("the token is gone", || async {
            let r = read(&tool).await;
            r.complete && r.keys.is_empty()
        })
        .await;
    }
    // Both listening, gossiping: they link, and the read finds it.
    let holder = common::peer(&ep, true, true).await;
    let reader = common::peer(&ep, true, true).await;
    let _token = holder.liveliness().declare_token(key).await.unwrap();
    eventually("the linked peer reads the token", || async {
        let r = read(&reader).await;
        r.complete && r.keys == [key]
    })
    .await;
}

// ─── a barrier that matches no token (#851, WB) ─────────────────────────────

/// A selector no zk2 token can match (#851): its `@zk` chunk is followed by
/// one chunk, where every token's is followed by its kind and at least one
/// chunk more (§1.1: `instance/<id>`, `alive/<iface>/<id>/<fp16>`,
/// `member/<iface>/<member>/<epoch>`).
const NO_TOKEN: &str = "zk2/*/*/@zk/settle";

/// The watch session's subscriber in these measurements: a reader's, with
/// history, on every instance token.
const INSTANCES: &str = "zk2/*/*/@zk/instance/*";

/// How long a watch session's link is held stalled from before it declares
/// its subscriber: longer than a seed's answer and a withdrawal take here,
/// as `spec/scenarios/presence.md` §9 step 5's.
const STALL: std::time::Duration = std::time::Duration::from_millis(150);

/// What a watch session heard: each (token key, put) in order.
type Heard = Arc<Mutex<Vec<(String, bool)>>>;

/// A liveliness subscriber on `watch`, with history or without, recording
/// what it hears.
async fn listen(watch: &zenoh::Session, history: bool) -> (zenoh::pubsub::Subscriber<()>, Heard) {
    let heard: Heard = Arc::default();
    let h = Arc::clone(&heard);
    let sub = presence::declare_subscriber(watch, INSTANCES)
        .history(history)
        .callback(move |s: Sample| {
            h.lock().unwrap().push((
                s.key_expr().as_str().to_owned(),
                s.kind() == SampleKind::Put,
            ));
        })
        .await
        .unwrap();
    (sub, heard)
}

/// zenoh's own liveliness GET of `sel`, as a barrier is made: whether it
/// ended at the final reply with no error reply, how many tokens it
/// carried, and how long it took.
async fn raw_barrier(
    s: &zenoh::Session,
    sel: &str,
    timeout: std::time::Duration,
) -> (bool, usize, std::time::Duration) {
    let started = std::time::Instant::now();
    let replies = s
        .liveliness()
        .get(sel)
        .timeout(timeout)
        .with(flume::unbounded())
        .await
        .unwrap();
    let (mut complete, mut tokens) = (true, 0);
    while let Ok(r) = replies.recv_async().await {
        match r.result() {
            Ok(_) => tokens += 1,
            Err(_) => complete = false,
        }
    }
    (complete, tokens, started.elapsed())
}

/// Whether `heard` holds `key`'s withdrawal within `within`.
async fn heard_gone(heard: &Heard, key: &str, within: std::time::Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        if heard
            .lock()
            .unwrap()
            .iter()
            .any(|(k, put)| k == key && !put)
        {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// One run of the race the barrier closes (core §8.1, 0.37; #851), its
/// window held open by a stall: a token `key` on `owner`, read by every
/// `tools` session first; a fresh watch session behind `link`, whose link
/// is stalled from before it declares its subscriber until [`STALL`] later.
/// With `settle`, the barrier ([`NO_TOKEN`]) is asked at once after the
/// declaration, and must still be waiting when the stall ends: the
/// session answered nothing itself. Then the seed (a GET at the token on
/// each tool), the token withdrawn at once, and the routers the tools read
/// shown it gone (a GET at the token). Without `settle`, the stall ends
/// only then, and a barrier made after it says the watch session's router
/// has read the subscriber. Returns whether the watch session heard the
/// token go. The subscriber has history when `history`.
async fn race(
    owner: &zenoh::Session,
    tools: &[&zenoh::Session],
    link: &common::Link,
    key: &str,
    settle: bool,
    history: bool,
) -> bool {
    let token = owner.liveliness().declare_token(key).await.unwrap();
    for tool in tools {
        eventually("the tool reads the token", || async {
            raw_read(tool, key).await.1 == [key]
        })
        .await;
    }
    let watch = common::retrying_client(link.endpoint()).await;
    link.stall();
    let (sub, heard) = if settle {
        let w = watch.clone();
        let starting = tokio::spawn(async move {
            let listening = listen(&w, history).await;
            let barrier = raw_barrier(&w, NO_TOKEN, std::time::Duration::from_secs(5)).await;
            (listening, barrier)
        });
        tokio::time::sleep(STALL).await;
        assert!(
            !starting.is_finished(),
            "the barrier came back while the link was stalled: answered by the session itself"
        );
        link.heal();
        let (listening, (complete, tokens, _)) = starting.await.unwrap();
        assert!(complete, "the barrier's final reply, no error reply");
        assert_eq!(tokens, 0, "the barrier matched no token");
        listening
    } else {
        listen(&watch, history).await
    };
    for tool in tools {
        assert_eq!(
            raw_read(tool, key).await.1,
            [key],
            "the seed holds the token"
        );
    }
    token.undeclare().await.unwrap();
    for tool in tools {
        eventually("the token is gone where the tool reads", || async {
            raw_read(tool, key).await.1.is_empty()
        })
        .await;
    }
    if !settle {
        link.heal();
        let (complete, _, _) =
            raw_barrier(&watch, NO_TOKEN, std::time::Duration::from_secs(5)).await;
        assert!(complete, "its router has read the subscriber now");
    }
    let gone = heard_gone(&heard, key, std::time::Duration::from_secs(2)).await;
    drop(sub);
    let _ = watch.close().await;
    gone
}

/// The race at full speed, many at once (#851): a fresh watch session
/// straight to the router at `ep`, its subscriber, the barrier when
/// `settle`, then the seed on `tool`, the token withdrawn at once, and the
/// tool shown it gone. Whether the watch session heard it go.
async fn race_unstalled(
    ep: &str,
    owner: &zenoh::Session,
    tool: &zenoh::Session,
    key: &str,
    settle: bool,
) -> bool {
    let token = owner.liveliness().declare_token(key).await.unwrap();
    eventually("the tool reads the token", || async {
        raw_read(tool, key).await.1 == [key]
    })
    .await;
    let watch = client(ep).await;
    let (sub, heard) = listen(&watch, true).await;
    if settle {
        let (complete, tokens, _) =
            raw_barrier(&watch, NO_TOKEN, std::time::Duration::from_secs(5)).await;
        assert!(
            complete && tokens == 0,
            "a barrier, empty: {complete} {tokens}"
        );
    }
    let seed = raw_read(tool, key).await.1;
    token.undeclare().await.unwrap();
    assert_eq!(seed, [key], "the seed holds the token");
    eventually("the token is gone where the tool reads", || async {
        raw_read(tool, key).await.1.is_empty()
    })
    .await;
    let gone = heard_gone(&heard, key, std::time::Duration::from_secs(2)).await;
    drop(sub);
    let _ = watch.close().await;
    gone
}

/// Declares `n` instance tokens on `holder` under system `system`, and
/// waits until `reader` reads them all: the load a barrier is made beside.
async fn crowd(
    holder: &zenoh::Session,
    reader: &zenoh::Session,
    system: &str,
    n: usize,
) -> Vec<zenoh::liveliness::LivelinessToken> {
    let mut tokens = Vec::with_capacity(n);
    for i in 0..n {
        tokens.push(
            holder
                .liveliness()
                .declare_token(format!("zk2/{system}/svc{i}/@zk/instance/{i:016x}"))
                .await
                .unwrap(),
        );
    }
    let sel = format!("zk2/{system}/*/@zk/instance/*");
    eventually("the crowd is read", || async {
        presence::liveliness_read(reader, &sel, std::time::Duration::from_secs(10))
            .await
            .is_ok_and(|r| r.complete && r.keys.len() == n)
    })
    .await;
    tokens
}

/// A client of `endpoint` whose runtime waits `interests_ms` for its
/// routers to finish a liveliness GET (`routing/interests/timeout`), and
/// that retries fast after losing `endpoint`.
async fn client_waiting(endpoint: &str, interests_ms: u64) -> zenoh::Session {
    client_waiting_in(endpoint, interests_ms, None).await
}

/// [`client_waiting`], in `namespace` when given.
async fn client_waiting_in(
    endpoint: &str,
    interests_ms: u64,
    namespace: Option<&str>,
) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    if let Some(ns) = namespace {
        c.insert_json5("namespace", &format!("\"{ns}\"")).unwrap();
    }
    for (k, v) in [
        ("mode", "\"client\"".to_owned()),
        ("scouting/multicast/enabled", "false".to_owned()),
        ("scouting/gossip/enabled", "false".to_owned()),
        ("connect/endpoints", format!("[\"{endpoint}\"]")),
        (
            "connect/retry",
            "{ period_init_ms: 100, period_max_ms: 500, period_increase_factor: 1.5 }".to_owned(),
        ),
        ("routing/interests/timeout", interests_ms.to_string()),
    ] {
        c.insert_json5(k, &v).unwrap();
    }
    zenoh::open(c).await.unwrap()
}

/// §10 (core §8.1, 0.41, 0.42; Appendix B; #851, #875), steps 1 to 5 and 7
/// to 9: a round trip whose GET matches no token orders a watch session's
/// subscriber at its router, on one router and through R2; a subscriber
/// without history hears an earlier token's withdrawal, except in a
/// namespace; beside 2,000 tokens in `ns` the round trip is one round trip
/// without history and waits for the history with it; a GET that runs to
/// its session's interests timeout ends with no error reply, which the
/// reference reads as possibly incomplete; one fresh session per
/// subscriber in a namespace (0.42, F-234); beside the crowd in `ns`, a
/// watch session in no namespace on the wire keys makes one round trip and
/// hears the withdrawal (0.42, F-233); and no round trip stands through a
/// watch session cut off, before it or while it runs (0.42, F-228). Step 6
/// observes the reader under test: `zenkey-fleet/tests/presence_scenarios.rs`'s
/// `s10_a_round_trip_at_a_population_and_the_interests_timeout`.
///
/// zenoh's code (`zenoh/src/net/routing`): a client's runtime forwards a
/// subscriber's interest and a GET's to its router on one link, at one
/// priority (`hat/client/interests.rs`), and the router handles each in
/// turn in `Face::interest` (`dispatcher/interests.rs`): the subscriber's
/// history is sent and its interest registered, then the GET's current
/// tokens — none — and its final reply, since a router resolves a client's
/// interest itself and ignores one from another router
/// (`hat/router/interests.rs`). A subscriber without history is told an
/// earlier token's withdrawal by key (`hat/broker/token.rs`), a key a
/// namespaced session does not strip (`routing/namespace.rs`). A GET its
/// routers have not finished is ended by the session's runtime at
/// `routing/interests/timeout`, with the final reply alone
/// (`CurrentInterestCleanup`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s10_a_round_trip_at_a_population_and_the_interests_timeout() {
    const RUNS: usize = 10;
    const CONTROLS: usize = 3;
    const CROWD: usize = 2_000;
    assert_eq!(
        presence::barrier_key(INSTANCES),
        NO_TOKEN,
        "the reference's"
    );
    let (r1, ep1) = router(None).await;
    let (_r2, ep2) = router(Some(&ep1)).await;
    let owner = client(&ep1).await;
    let tool = client(&ep1).await;
    let tool2 = client(&ep2).await;

    // 1, 2: one router.
    let link1 = common::link_to(&ep1).await;
    let started = std::time::Instant::now();
    let mut heard = 0;
    for run in 0..RUNS {
        let key = format!("zk2/s10/run{run}/@zk/instance/{run:016x}");
        heard += usize::from(race(&owner, &[&tool], &link1, &key, true, false).await);
    }
    eprintln!(
        "§10 step 1: heard {heard} of {RUNS} in {:?}",
        started.elapsed()
    );
    assert_eq!(heard, RUNS, "a withdrawal after the round trip was missed");
    let mut missed = 0;
    for run in 0..CONTROLS {
        let key = format!("zk2/s10/control{run}/@zk/instance/{run:016x}");
        missed += usize::from(!race(&owner, &[&tool], &link1, &key, false, false).await);
    }
    eprintln!("§10 step 2: missed {missed} of {CONTROLS}");
    assert_eq!(missed, CONTROLS, "the stall should hold the race open");

    // 3: through R2, the seed read at R2 and at R1.
    let link2 = common::link_to(&ep2).await;
    let mut heard = 0;
    for run in 0..RUNS {
        let key = format!("zk2/s10/r2run{run}/@zk/instance/{run:016x}");
        heard += usize::from(race(&owner, &[&tool2, &tool], &link2, &key, true, false).await);
    }
    let mut missed = 0;
    for run in 0..CONTROLS {
        let key = format!("zk2/s10/r2control{run}/@zk/instance/{run:016x}");
        missed += usize::from(!race(&owner, &[&tool2, &tool], &link2, &key, false, false).await);
    }
    eprintln!("§10 step 3: heard {heard} of {RUNS}, missed {missed} of {CONTROLS} without");
    assert_eq!(heard, RUNS, "a withdrawal after the round trip was missed");
    assert_eq!(missed, CONTROLS, "the stall should hold the race open");

    // 4: beside the crowd, which is in `ns` (0.42): watch sessions in `ns`.
    let tool_ns = common::client_ns(&ep1, Some("ns")).await;
    let holders = [
        common::client_ns(&ep1, Some("ns")).await,
        common::client_ns(&ep1, Some("ns")).await,
    ];
    let started = std::time::Instant::now();
    let mut crowd_held = {
        let mut held = Vec::new();
        for (i, h) in holders.iter().enumerate() {
            held.extend(crowd(h, &tool_ns, &format!("crowd{i}"), CROWD / holders.len()).await);
        }
        held
    };
    eprintln!("§10 step 4: the crowd's setup {:?}", started.elapsed());
    let watch = common::client_ns(&ep1, Some("ns")).await;
    let (_sub, heard) = listen(&watch, false).await;
    let (complete, tokens, took) =
        raw_barrier(&watch, NO_TOKEN, std::time::Duration::from_secs(5)).await;
    eprintln!("§10 step 4: without history, {took:?}, {tokens} tokens");
    assert!(complete && tokens == 0, "{complete} {tokens}");
    assert!(
        took < std::time::Duration::from_secs(1),
        "one round trip beside {CROWD} tokens: {took:?}"
    );
    assert!(heard.lock().unwrap().is_empty(), "no history");
    let watch = client_waiting_in(&ep1, 60_000, Some("ns")).await;
    let (_sub, heard) = listen(&watch, true).await;
    let (complete, tokens, took) =
        raw_barrier(&watch, NO_TOKEN, std::time::Duration::from_secs(30)).await;
    let by_then = heard.lock().unwrap().len();
    eprintln!("§10 step 4: with history, {took:?}, {by_then} heard by its end");
    assert!(complete && tokens == 0, "{complete} {tokens}");
    assert!(
        by_then >= CROWD,
        "the round trip follows the history: {by_then}"
    );

    // 5: the interests timeout.
    let link5 = common::link_to(&ep1).await;
    let reader = client_waiting(link5.endpoint(), 300).await;
    assert_eq!(
        presence::interests_timeout(&reader),
        std::time::Duration::from_millis(300)
    );
    // The steps' own tokens, all withdrawn: beside the crowd, a read of
    // every instance token takes longer than 300 ms, and is possibly
    // incomplete for that alone.
    let narrow = "zk2/s10/*/@zk/instance/*";
    let open = presence::liveliness_read(&reader, narrow, std::time::Duration::from_secs(2))
        .await
        .unwrap();
    assert!(open.complete && !open.at_interests_timeout, "{open:?}");
    link5.stall();
    let (no_error, tokens, took) =
        raw_barrier(&reader, narrow, std::time::Duration::from_secs(2)).await;
    eprintln!(
        "§10 step 5: zenoh's own GET, stalled: no error reply {no_error}, {tokens} tokens, {took:?}"
    );
    assert!(no_error, "zenoh ends it with the final reply alone");
    assert!(
        took < std::time::Duration::from_secs(2),
        "at the interests timeout: {took:?}"
    );
    let held = presence::liveliness_read(&reader, narrow, std::time::Duration::from_secs(2))
        .await
        .unwrap();
    eprintln!("§10 step 5: the reference's read, stalled: {held:?}");
    assert!(!held.complete, "never complete: {held:?}");
    assert!(
        held.errors.is_empty() && held.at_interests_timeout,
        "{held:?}"
    );
    assert_eq!(
        held.incomplete_reason(),
        Some(presence::AT_INTERESTS_TIMEOUT)
    );
    let settled = presence::settle(&reader, INSTANCES, std::time::Duration::from_secs(2))
        .await
        .unwrap();
    assert!(
        !settled,
        "a round trip ended by the runtime establishes nothing"
    );
    link5.heal();

    // 7: a namespace. The owner and the tool in `ns`; the watch session in
    // `ns`, without history and with it, or in none, on the wire key.
    let owner = common::client_ns(&ep1, Some("ns")).await;
    let tool = common::client_ns(&ep1, Some("ns")).await;
    let mut heard_by = Vec::new();
    for (ns, history) in [(Some("ns"), false), (Some("ns"), true), (None, false)] {
        let key = format!(
            "zk2/s10/{}{history}/@zk/instance/000000000000000a",
            ns.unwrap_or("wire")
        );
        let token = owner.liveliness().declare_token(&key).await.unwrap();
        eventually("the tool reads it", || async {
            raw_read(&tool, &key).await.1 == [key.clone()]
        })
        .await;
        let watch = common::client_ns(&ep1, ns).await;
        assert_eq!(presence::subscriber_needs_history(&watch), ns.is_some());
        // The steps' own instance tokens: the crowd's are in `ns` too.
        let own = "zk2/s10/*/@zk/instance/*";
        let sel = match ns {
            Some(_) => own.to_owned(),
            None => format!("ns/{own}"),
        };
        let heard: Heard = Arc::default();
        let h = Arc::clone(&heard);
        let _sub = presence::declare_subscriber(&watch, &sel)
            .history(history)
            .callback(move |s: Sample| {
                h.lock().unwrap().push((
                    s.key_expr().as_str().to_owned(),
                    s.kind() == SampleKind::Put,
                ));
            })
            .await
            .unwrap();
        assert!(presence::settle(&watch, &sel, T).await.unwrap());
        token.undeclare().await.unwrap();
        eventually("gone where the tool reads", || async {
            raw_read(&tool, &key).await.1.is_empty()
        })
        .await;
        let wire = match ns {
            Some(_) => key.clone(),
            None => format!("ns/{key}"),
        };
        let gone = heard_gone(&heard, &wire, std::time::Duration::from_secs(1)).await;
        heard_by.push((ns, history, gone));
    }
    eprintln!("§10 step 7: (namespace, history, heard going) {heard_by:?}");
    assert_eq!(
        heard_by,
        [
            (Some("ns"), false, false),
            (Some("ns"), true, true),
            (None, false, true)
        ],
        "zenoh fixed the namespace's withdrawal by key: revisit core §8.1 (0.41) and \
         `subscriber_needs_history`"
    );

    // 8 (0.42, F-233): beside the crowd in `ns`, a fresh watch session in no
    // namespace, on the wire keys, without history.
    let watch = client(&ep1).await;
    let heard: Heard = Arc::default();
    let h = Arc::clone(&heard);
    let _sub = presence::declare_subscriber(&watch, format!("ns/{INSTANCES}"))
        .history(false)
        .callback(move |s: Sample| {
            h.lock().unwrap().push((
                s.key_expr().as_str().to_owned(),
                s.kind() == SampleKind::Put,
            ));
        })
        .await
        .unwrap();
    let (complete, tokens, took) = raw_barrier(
        &watch,
        &format!("ns/{NO_TOKEN}"),
        std::time::Duration::from_secs(5),
    )
    .await;
    assert!(complete && tokens == 0, "{complete} {tokens}");
    assert!(
        took < std::time::Duration::from_secs(1),
        "one round trip beside {CROWD} tokens in ns, from no namespace: {took:?}"
    );
    let gone = "zk2/crowd0/svc0/@zk/instance/0000000000000000";
    crowd_held.remove(0).undeclare().await.unwrap();
    let wire = format!("ns/{gone}");
    let heard_going = heard_gone(&heard, &wire, std::time::Duration::from_secs(2)).await;
    eprintln!(
        "§10 step 8: from no namespace on the wire keys, {took:?}, {tokens} tokens; the crowd \
         token's withdrawal heard {heard_going}"
    );
    assert!(heard_going, "heard on its wire key");
    drop(_sub);
    let _ = watch.close().await;

    // 9 (0.42, F-228): no round trip stands through a watch session cut
    // off: none is asked before it, and one cut while it waits comes back
    // at the cut with the final reply alone.
    let link9 = common::link_to(&ep1).await;
    let watch = common::retrying_client(link9.endpoint()).await;
    let (_sub, _) = listen(&watch, false).await;
    assert!(presence::settle(&watch, INSTANCES, T).await.unwrap());
    link9.cut();
    eventually("the watch session cut off", || presence::cut_off(&watch)).await;
    let asked = presence::liveliness_gets(&watch).settles;
    let cut = presence::settle(&watch, INSTANCES, std::time::Duration::from_secs(2))
        .await
        .unwrap();
    assert!(
        !cut,
        "a round trip through a session cut off does not stand"
    );
    assert_eq!(
        presence::liveliness_gets(&watch).settles,
        asked,
        "and is not asked"
    );
    link9.heal();
    eventually("the watch session reconnected", || async {
        !presence::cut_off(&watch).await
    })
    .await;
    let mut ended = Vec::new();
    for reference in [false, true] {
        link9.stall();
        let w = watch.clone();
        let asking = tokio::spawn(async move {
            let t = std::time::Instant::now();
            let stood = if reference {
                presence::settle(&w, INSTANCES, std::time::Duration::from_secs(2))
                    .await
                    .unwrap()
            } else {
                let (complete, tokens, _) =
                    raw_barrier(&w, NO_TOKEN, std::time::Duration::from_secs(2)).await;
                assert_eq!(tokens, 0, "the final reply alone");
                complete
            };
            (stood, t.elapsed())
        });
        tokio::time::sleep(STALL).await;
        assert!(!asking.is_finished(), "still waiting while stalled");
        link9.cut();
        let (stood, took) = asking.await.unwrap();
        assert!(
            took < std::time::Duration::from_secs(2),
            "answered at the cut, not at its timeout: {took:?}"
        );
        ended.push((reference, stood, took));
        link9.heal();
        eventually("the watch session reconnected", || async {
            !presence::cut_off(&watch).await
        })
        .await;
    }
    eprintln!("§10 step 9: (the reference, no error reply / stands, took) {ended:?}");
    assert!(
        ended[0].1,
        "zenoh's own GET cut while it ran: the final reply, no error reply"
    );
    assert!(
        !ended[1].1,
        "the reference's round trip cut while it ran does not stand"
    );

    // 10 (0.44, F-239): beside the crowd in `ns`, a watch session in no
    // namespace through a link the test cuts and heals. The reopen tells
    // its subscriber without history every crowd token, the one declared
    // while the link was cut included and the one withdrawn then not, and
    // the round trip after it waits for them; the reference asks one round
    // trip for it, however often it is asked while it runs (#882).
    let link10 = common::link_to(&ep1).await;
    let watch = client_waiting(link10.endpoint(), 60_000).await;
    let heard: Heard = Arc::default();
    let h = Arc::clone(&heard);
    let _sub = presence::declare_subscriber(&watch, format!("ns/{INSTANCES}"))
        .history(false)
        .callback(move |s: Sample| {
            h.lock().unwrap().push((
                s.key_expr().as_str().to_owned(),
                s.kind() == SampleKind::Put,
            ));
        })
        .await
        .unwrap();
    let barrier = presence::Barrier::at(
        &watch,
        &format!("ns/{NO_TOKEN}"),
        std::time::Duration::from_secs(5),
    )
    .await
    .unwrap();
    let t = std::time::Instant::now();
    assert!(barrier.establish().await, "the round trip at the start");
    let at_start = t.elapsed();
    assert!(
        at_start < std::time::Duration::from_secs(1),
        "one round trip at the start: {at_start:?}"
    );
    assert!(heard.lock().unwrap().is_empty(), "no history");
    let seen = barrier.moves();
    link10.cut();
    eventually("the watch session cut off", || presence::cut_off(&watch)).await;
    // While the link is cut: one crowd token withdrawn, one declared.
    let withdrawn = "zk2/crowd0/svc1/@zk/instance/0000000000000001";
    crowd_held.remove(0).undeclare().await.unwrap();
    let born = "zk2/crowd0/born/@zk/instance/00000000000000bb";
    let _born = holders[0].liveliness().declare_token(born).await.unwrap();
    eventually("the router holds the move", || async {
        raw_read(&tool_ns, born).await.1 == [born.to_owned()]
            && raw_read(&tool_ns, withdrawn).await.1.is_empty()
    })
    .await;
    let told_at_cut = heard.lock().unwrap().clone();
    heard.lock().unwrap().clear();
    let asked = presence::liveliness_gets(&watch).settles;
    link10.heal();
    eventually("the watch session's transport opened again", || async {
        barrier.moves() >= seen + 2
    })
    .await;
    let reopened = std::time::Instant::now();
    for _ in 0..3 {
        let _ = barrier
            .establish_within(std::time::Duration::from_millis(1))
            .await;
    }
    let stood = barrier
        .establish_within(std::time::Duration::from_secs(30))
        .await;
    let after = reopened.elapsed();
    let told: BTreeSet<String> = heard
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, put)| *put)
        .map(|(k, _)| k.clone())
        .collect();
    let trips = presence::liveliness_gets(&watch).settles - asked;
    eprintln!(
        "§10 step 10: the round trip at the start {at_start:?}, carrying none; after the \
         reopen {after:?}, standing {stood}, {} tokens told by its end, {trips} round trip(s) \
         asked",
        told.len()
    );
    assert!(
        told_at_cut.is_empty(),
        "nothing told at the cut: {told_at_cut:?}"
    );
    assert!(stood, "the round trip after the reopen stands");
    assert_eq!(trips, 1, "one round trip, however often it was asked");
    assert!(
        told.contains(&format!("ns/{born}")),
        "the token declared while the link was cut is told at the reopen"
    );
    assert!(
        !heard
            .lock()
            .unwrap()
            .iter()
            .any(|(k, _)| *k == format!("ns/{withdrawn}")),
        "the token withdrawn while the link was cut is never told"
    );
    let held = CROWD - 2 + 1;
    assert_eq!(
        told.len(),
        held,
        "every crowd token held, told by the round trip's end"
    );
    drop(_sub);
    let _ = watch.close().await;
    drop(r1);
}

/// #872 (WC), measured to decide where a watch session belongs in a
/// namespaced deployment. Beside 2,000 instance tokens declared in `ns`
/// before it, three shapes of watch session, each timed over [`RUNS`]
/// fresh sessions: its subscriber, its round trip ([`NO_TOKEN`] in its own
/// key space), and one crowd token withdrawn after it.
///
/// - **In `ns`, with history**, what the readers do since #872: the round
///   trip waits for 2,000 puts, and the withdrawal is heard.
/// - **In no namespace, on the wire key `ns/zk2/…`, without history**: one
///   round trip, and the withdrawal is heard, by its wire key, which the
///   reader would strip of `ns/` itself.
/// - **In `ns`, without history**, the control: one round trip, and the
///   withdrawal is never heard (§10 step 7).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_watch_session_in_a_namespace_waits_for_its_history() {
    const CROWD: usize = 2_000;
    const RUNS: usize = 3;
    let (_r, ep) = router(None).await;
    let tool = common::client_ns(&ep, Some("ns")).await;
    let holders = [
        common::client_ns(&ep, Some("ns")).await,
        common::client_ns(&ep, Some("ns")).await,
    ];
    let started = std::time::Instant::now();
    let mut crowd: Vec<(String, zenoh::liveliness::LivelinessToken)> = Vec::new();
    for i in 0..CROWD {
        let key = format!("zk2/crowd/svc{i}/@zk/instance/{:016x}", i + 1);
        let token = holders[i % 2]
            .liveliness()
            .declare_token(&key)
            .await
            .unwrap();
        crowd.push((key, token));
    }
    eventually("the crowd is read", || async {
        presence::liveliness_read(
            &tool,
            "zk2/crowd/*/@zk/instance/*",
            std::time::Duration::from_secs(10),
        )
        .await
        .is_ok_and(|r| r.complete && r.keys.len() == CROWD)
    })
    .await;
    eprintln!("#872: the crowd's setup {:?}", started.elapsed());
    let mut seen = Vec::new();
    for (ns, history) in [(Some("ns"), true), (None, false), (Some("ns"), false)] {
        let (mut took, mut by_end, mut gone) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..RUNS {
            let watch = watch_in(&ep, ns).await;
            let wire = |k: &str| match ns {
                Some(_) => k.to_owned(),
                None => format!("ns/{k}"),
            };
            let heard: Heard = Arc::default();
            let h = Arc::clone(&heard);
            let sub = presence::declare_subscriber(&watch, wire(INSTANCES))
                .history(history)
                .callback(move |s: Sample| {
                    h.lock().unwrap().push((
                        s.key_expr().as_str().to_owned(),
                        s.kind() == SampleKind::Put,
                    ));
                })
                .await
                .unwrap();
            let (complete, tokens, t) =
                raw_barrier(&watch, &wire(NO_TOKEN), std::time::Duration::from_secs(60)).await;
            assert!(
                complete && tokens == 0,
                "{ns:?} {history}: {complete} {tokens}"
            );
            took.push(t);
            by_end.push(heard.lock().unwrap().len());
            let (key, token) = crowd.pop().expect("a crowd token");
            token.undeclare().await.unwrap();
            eventually("gone where the tool reads", || async {
                raw_read(&tool, &key).await.1.is_empty()
            })
            .await;
            gone.push(heard_gone(&heard, &wire(&key), std::time::Duration::from_secs(1)).await);
            drop(sub);
            let _ = watch.close().await;
        }
        eprintln!(
            "#872: watch session in {ns:?}, history {history}: round trip {took:?}, heard by \
             its end {by_end:?}, an earlier token's withdrawal heard {gone:?}"
        );
        seen.push((ns, history, took, by_end, gone));
    }
    let second = std::time::Duration::from_secs(1);
    let [with, wire, without] = &seen[..] else {
        panic!("three shapes")
    };
    assert!(
        with.3.iter().all(|n| *n >= CROWD - RUNS) && with.4.iter().all(|g| *g),
        "in ns with history: the round trip follows the history, and hears it go: {with:?}"
    );
    assert!(
        wire.2.iter().all(|t| *t < second) && wire.3.iter().all(|n| *n == 0),
        "in no namespace without history: one round trip: {wire:?}"
    );
    assert!(wire.4.iter().all(|g| *g), "and it hears it go: {wire:?}");
    assert!(
        without.2.iter().all(|t| *t < second) && without.4.iter().all(|g| !g),
        "zenoh fixed the namespace's withdrawal by key: revisit `subscriber_needs_history` \
         and #872: {without:?}"
    );
}

/// A watch session for [`a_watch_session_in_a_namespace_waits_for_its_history`]:
/// in `namespace` when given, its runtime waiting 120 s for its routers to
/// finish a liveliness GET, as a tool's session does since #851.
async fn watch_in(endpoint: &str, namespace: Option<&str>) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    for (k, v) in [
        ("mode", "\"client\"".to_owned()),
        ("scouting/multicast/enabled", "false".to_owned()),
        ("scouting/gossip/enabled", "false".to_owned()),
        ("connect/endpoints", format!("[\"{endpoint}\"]")),
        ("routing/interests/timeout", "120000".to_owned()),
    ] {
        c.insert_json5(k, &v).unwrap();
    }
    if let Some(ns) = namespace {
        c.insert_json5("namespace", &format!("\"{ns}\"")).unwrap();
    }
    zenoh::open(c).await.unwrap()
}

/// #851, under load: the round trip, many at once beside 300 tokens a
/// history subscriber carries, its link not stalled. With it no withdrawal
/// is missed; without it some are, which is the race it closes, too narrow
/// to hit every time without a stall (§10's control holds it open).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_round_trip_under_load_misses_no_withdrawal() {
    const UNSTALLED: usize = 160;
    const AT_ONCE: usize = 8;
    const CROWD: usize = 300;
    let (_r1, ep) = router(None).await;
    let owner = client(&ep).await;
    let tool = client(&ep).await;
    let holder = client(&ep).await;
    let _crowd = crowd(&holder, &tool, "crowd", CROWD).await;
    let ep = Arc::new(ep);
    let (owner, tool) = (Arc::new(owner), Arc::new(tool));
    for settle in [true, false] {
        let started = std::time::Instant::now();
        let mut missed = 0;
        for batch in 0..UNSTALLED / AT_ONCE {
            let runs: Vec<_> = (0..AT_ONCE)
                .map(|i| {
                    let (ep, owner, tool) =
                        (Arc::clone(&ep), Arc::clone(&owner), Arc::clone(&tool));
                    let n = batch * AT_ONCE + i;
                    tokio::spawn(async move {
                        let key = format!("zk2/wb/u{settle}{n}/@zk/instance/{n:016x}");
                        race_unstalled(&ep, &owner, &tool, &key, settle).await
                    })
                })
                .collect();
            for r in runs {
                missed += usize::from(!r.await.unwrap());
            }
        }
        eprintln!(
            "{AT_ONCE} at once beside {CROWD} tokens, round trip {settle}: missed {missed} of \
             {UNSTALLED} in {:?}",
            started.elapsed()
        );
        if settle {
            assert_eq!(missed, 0, "a withdrawal after the round trip was missed");
        }
    }
}

/// #851: a barrier the router's access control refuses is still ordered
/// (zenoh 1.10.1). A refused interest is answered with its final reply at
/// ingress (`net/primitives/demux.rs`), after every earlier message of the
/// face was handled, so it orders the subscriber as a granted one does;
/// what a grant buys is only that nothing is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_barrier_still_orders_the_subscriber() {
    const RUNS: usize = 5;
    let acl = r#"{
        enabled: true,
        default_permission: "allow",
        rules: [{
            id: "no-barrier",
            messages: ["liveliness_query"],
            flows: ["ingress"],
            permission: "deny",
            key_exprs: ["zk2/*/*/@zk/settle"],
        }],
        subjects: [{ id: "anyone" }],
        policies: [{ rules: ["no-barrier"], subjects: ["anyone"] }],
    }"#;
    let (r1, ep) = common::router_with(None, &[("access_control", acl)]).await;
    let owner = client(&ep).await;
    let tool = client(&ep).await;
    let link = common::link_to(&ep).await;
    // A token the barrier would carry if it were answered: the router's
    // own read, through no face, holds it, and every barrier below comes
    // back empty, refused.
    let check = "zk2/wbr/check/@zk/settle";
    let _check = owner.liveliness().declare_token(check).await.unwrap();
    eventually("the router holds the check token", || async {
        keys(&r1, NO_TOKEN).await == [check]
    })
    .await;
    assert!(raw_read(&tool, NO_TOKEN).await.1.is_empty(), "refused");
    let mut heard = 0;
    for run in 0..RUNS {
        let key = format!("zk2/wbr/run{run}/@zk/instance/{run:016x}");
        heard += usize::from(race(&owner, &[&tool], &link, &key, true, true).await);
    }
    eprintln!("one router, the barrier refused: heard {heard} of {RUNS}");
    assert_eq!(heard, RUNS);
}

/// #882: a barrier's round trip runs on a task of its own, one per
/// transport state, that every waiter shares. Through a stalled link,
/// three waiters that stop waiting and one dropped at its await ask one
/// round trip between them; when the stall ends that one stands, and
/// nothing more is asked. A transport move loses it, and the next waiter
/// asks a new one. Before, each waiter asked a GET of its own, and a GET
/// its caller stops waiting for is still answered in full: zenoh 1.10.1
/// tells the router nothing of it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_round_trip_in_flight_is_taken_up_not_asked_again() {
    use std::time::Duration;
    let (_r1, ep) = router(None).await;
    let link = common::link_to(&ep).await;
    let watch = client_waiting(link.endpoint(), 30_000).await;
    let (_sub, _) = listen(&watch, false).await;
    let barrier = presence::Barrier::new(&watch, INSTANCES, Duration::from_millis(50))
        .await
        .unwrap();
    let asked = presence::liveliness_gets(&watch).settles;

    link.stall();
    for _ in 0..3 {
        assert!(!barrier.establish().await, "not back while stalled");
    }
    let dropped =
        tokio::time::timeout(Duration::from_millis(20), barrier.establish_within(T)).await;
    assert!(dropped.is_err(), "dropped at its await");
    assert!(barrier.in_flight(), "the round trip runs on");
    assert_eq!(
        presence::liveliness_gets(&watch).settles - asked,
        1,
        "one round trip between them"
    );
    link.heal();
    assert!(
        barrier.establish_within(Duration::from_secs(5)).await,
        "the same round trip stands once the stall ends"
    );
    assert!(!barrier.in_flight());
    assert_eq!(presence::liveliness_gets(&watch).settles - asked, 1);

    // A move loses it; the next waiter asks a new one, which stands. The
    // waiter asks it once both moves are counted, the close and the open:
    // the open's event can follow the links by a moment, and a round trip
    // asked before it is one the waiter leaves for the new state's (a
    // second one asked, as the move rule wants).
    let seen = barrier.moves();
    link.cut();
    eventually("the watch session cut off", || presence::cut_off(&watch)).await;
    link.heal();
    eventually("the watch session's transport opened again", || async {
        barrier.moves() >= seen + 2
    })
    .await;
    assert!(!barrier.established(), "lost with the transport");
    assert!(barrier.establish_within(Duration::from_secs(5)).await);
    assert_eq!(presence::liveliness_gets(&watch).settles - asked, 2);
}

/// F-268 measured in Rust (`arbitration.v1` text 0.2, core 0.56, #909's
/// BB): on zenoh 1.10.1, when a watch session's transport closes, which
/// does it hear first, the close's withdrawals on its liveliness
/// subscriber, or the close on its transport events listener? zk2py
/// measured the withdrawals 0.15–0.5 ms before the event on zenoh-python
/// 1.10.1.
///
/// A token holder through R1 declares three interface tokens; the watch
/// session, a client of R1 through a link the test cuts and heals, holds a
/// subscriber without history and a transport events listener, and records
/// each callback's instant; a task the subscriber's callback spawns reads
/// whether the session still holds a link. The first cut tells nothing (a
/// subscriber without history was told of no token, core §8.1, 0.41); the
/// reopen tells every token (0.44); each later cut tells the three
/// withdrawals. [`F268_CUTS`] cuts, each one's order asserted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn f268_a_close_tells_its_withdrawals_before_its_transport_event() {
    use std::time::{Duration, Instant};
    let (_r1, ep) = router(None).await;
    let holder = client(&ep).await;
    let keys: Vec<String> = (1..=3)
        .map(|i| format!("zk2/lab/safety/@zk/alive/steer.v1/000000000000000{i}/0123456789abcdef"))
        .collect();
    let mut _tokens = Vec::new();
    for k in &keys {
        _tokens.push(holder.liveliness().declare_token(k.as_str()).await.unwrap());
    }
    let link = common::link_to(&ep).await;
    let watch = common::retrying_client(link.endpoint()).await;
    // (instant, put, key) heard; (instant, opened) per transport event;
    // (instant, links held) read after each withdrawal.
    type Told = Arc<Mutex<Vec<(Instant, bool, String)>>>;
    let told: Told = Arc::default();
    let events: Arc<Mutex<Vec<(Instant, bool)>>> = Arc::default();
    let links: Arc<Mutex<Vec<(Instant, bool)>>> = Arc::default();
    let e = Arc::clone(&events);
    let _listener = watch
        .info()
        .transport_events_listener()
        .callback(move |ev| {
            let at = Instant::now();
            let opened = ev.kind() == SampleKind::Put;
            e.lock().unwrap().push((at, opened));
        })
        .await
        .unwrap();
    let (t, l, w, rt) = (
        Arc::clone(&told),
        Arc::clone(&links),
        watch.clone(),
        tokio::runtime::Handle::current(),
    );
    let _sub = presence::declare_subscriber(&watch, "zk2/lab/*/@zk/alive/**")
        .history(false)
        .callback(move |s: Sample| {
            let at = Instant::now();
            let put = s.kind() == SampleKind::Put;
            t.lock()
                .unwrap()
                .push((at, put, s.key_expr().as_str().to_owned()));
            if !put {
                let (l, w) = (Arc::clone(&l), w.clone());
                rt.spawn(async move {
                    let held = !presence::cut_off(&w).await;
                    l.lock().unwrap().push((at, held));
                });
            }
        })
        .await
        .unwrap();
    assert!(
        presence::settle(&watch, "zk2/lab/*/@zk/alive/**", T)
            .await
            .unwrap()
    );
    let mut rows = Vec::new();
    for cut in 0..F268_CUTS {
        let (told_before, events_before) =
            (told.lock().unwrap().len(), events.lock().unwrap().len());
        link.cut();
        eventually("the close's transport event", || async {
            events.lock().unwrap()[events_before..]
                .iter()
                .any(|(_, o)| !o)
        })
        .await;
        if cut > 0 {
            eventually("the close's three withdrawals", || async {
                told.lock().unwrap()[told_before..]
                    .iter()
                    .filter(|(_, put, _)| !put)
                    .count()
                    == keys.len()
            })
            .await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let closed = events.lock().unwrap()[events_before..]
            .iter()
            .find(|(_, o)| !o)
            .map(|(at, _)| *at)
            .unwrap();
        let gone: Vec<Instant> = told.lock().unwrap()[told_before..]
            .iter()
            .filter(|(_, put, _)| !put)
            .map(|(at, ..)| *at)
            .collect();
        if cut == 0 {
            assert!(gone.is_empty(), "the first close tells nothing: {gone:?}");
        } else {
            let first = *gone.iter().min().unwrap();
            let last = *gone.iter().max().unwrap();
            let ahead = |a: Instant| -> f64 {
                if a <= closed {
                    closed.duration_since(a).as_secs_f64() * 1e3
                } else {
                    -(a.duration_since(closed).as_secs_f64() * 1e3)
                }
            };
            rows.push((cut, ahead(first), ahead(last)));
        }
        let told_at_heal = told.lock().unwrap().len();
        link.heal();
        eventually("the reopen tells every token", || async {
            told.lock().unwrap()[told_at_heal..]
                .iter()
                .filter(|(_, put, _)| *put)
                .count()
                >= keys.len()
        })
        .await;
        eventually("the session holds a link again", || async {
            !presence::cut_off(&watch).await
        })
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let held: Vec<bool> = links.lock().unwrap().iter().map(|(_, h)| *h).collect();
    eprintln!("F-268: (cut, first withdrawal ms before the close event, last) {rows:?}");
    eprintln!("F-268: a link held when each withdrawal's check ran: {held:?}");
    assert_eq!(rows.len(), F268_CUTS - 1);
    for (cut, first, last) in &rows {
        assert!(
            *first > 0.0 && *last > 0.0,
            "cut {cut}: every withdrawal heard before the close's transport event \
             ({first:.3} ms, {last:.3} ms before it)"
        );
    }
    assert!(
        held.iter().all(|h| !h),
        "no link held when a close's withdrawal was heard: {held:?}"
    );
}

/// F-268's cuts: the first tells nothing, the five after it each three
/// withdrawals.
const F268_CUTS: usize = 6;

/// What `arbitration.v1`'s runtime found (text 0.2, core 0.56, #909's
/// BB): zenoh 1.10.1 tells a session's transport events listener that a
/// transport opened before its router re-declares the session's
/// subscribers' interests on it (`net/runtime/mod.rs`, `new_unicast`: the
/// session's handlers, the listener's among them, then the router's
/// `new_transport_unicast`, which re-propagates them). A round trip asked
/// at once at the open can find no face to go to, and is answered at
/// once, complete and empty, before the reopen tells the subscribers a
/// token: its standing says nothing. One asked 100 ms after the open
/// follows the reopen's telling, every time.
///
/// A watch session through a link the test cuts and heals, its three
/// subscribers without history over three tokens. [`OPEN_ROUNDS`] reopens;
/// at each, a round trip asked at the open event, then one 100 ms after it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_round_trip_asked_at_the_open_event_can_precede_the_reopen() {
    use std::time::{Duration, Instant};
    let (_r1, ep) = router(None).await;
    let holder = client(&ep).await;
    let mut _tokens = Vec::new();
    for i in 1..=3 {
        _tokens.push(
            holder
                .liveliness()
                .declare_token(format!(
                    "zk2/lab/s{i}/@zk/alive/steer.v1/000000000000000{i}/0123456789abcdef"
                ))
                .await
                .unwrap(),
        );
    }
    let link = common::link_to(&ep).await;
    let watch = common::retrying_client(link.endpoint()).await;
    let told: Arc<Mutex<Vec<Instant>>> = Arc::default();
    let mut _subs = Vec::new();
    for i in 1..=3 {
        let t = Arc::clone(&told);
        _subs.push(
            presence::declare_subscriber(&watch, format!("zk2/lab/s{i}/@zk/alive/**"))
                .history(false)
                .callback(move |s: Sample| {
                    if s.kind() == SampleKind::Put {
                        t.lock().unwrap().push(Instant::now());
                    }
                })
                .await
                .unwrap(),
        );
    }
    let key = "zk2/lab/s1/@zk/settle";
    assert!(presence::settle_at(&watch, key, T).await.unwrap());
    let (tx, rx) = flume::unbounded::<Instant>();
    let _listener = watch
        .info()
        .transport_events_listener()
        .callback(move |ev| {
            if ev.kind() == SampleKind::Put {
                let _ = tx.send(Instant::now());
            }
        })
        .await
        .unwrap();
    let mut at_once_before = 0;
    for round in 0..OPEN_ROUNDS {
        link.cut();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let from = told.lock().unwrap().len();
        link.heal();
        let opened = rx.recv_async().await.unwrap();
        // At once at the open event.
        assert!(presence::settle_at(&watch, key, T).await.unwrap());
        let replied = Instant::now();
        let early = told.lock().unwrap()[from..]
            .iter()
            .filter(|at| **at <= replied)
            .count();
        // 100 ms after it.
        tokio::time::sleep_until((opened + Duration::from_millis(100)).into()).await;
        assert!(presence::settle_at(&watch, key, T).await.unwrap());
        let later = Instant::now();
        let after = told.lock().unwrap()[from..]
            .iter()
            .filter(|at| **at <= later)
            .count();
        assert_eq!(
            after, 3,
            "round {round}: the reopen's telling before the later reply"
        );
        if early < 3 {
            at_once_before += 1;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    eprintln!(
        "a round trip asked at the open event stood before the reopen's telling at \
         {at_once_before} of {OPEN_ROUNDS} reopens"
    );
}

/// The reopens of the test above.
const OPEN_ROUNDS: usize = 10;

/// #914: the barrier asks the round trip after an open a margin after it
/// (core §8.1, 0.56: "A reader SHOULD ask the round trip after an open a
/// margin after its event (the reference: 100 ms)"). The test above's
/// watch session, its three subscribers without history over three tokens,
/// and a [`presence::Barrier`] on it. [`OPEN_ROUNDS`] reopens; at each the
/// reader asks the barrier at once at the open, as a reader that follows
/// [`presence::Barrier::moved_since`] does, and its round trip stands only
/// after the reopen has told every token, asked no sooner than
/// [`presence::OPEN_MARGIN`] after the open. Then a move within the margin:
/// the round trip waiting it out ends unasked, standing not.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_barrier_round_trip_after_an_open_follows_the_reopen() {
    use std::time::{Duration, Instant};
    let (_r1, ep) = router(None).await;
    let holder = client(&ep).await;
    let mut _tokens = Vec::new();
    for i in 1..=3 {
        _tokens.push(
            holder
                .liveliness()
                .declare_token(format!(
                    "zk2/lab/s{i}/@zk/alive/steer.v1/000000000000000{i}/0123456789abcdef"
                ))
                .await
                .unwrap(),
        );
    }
    let link = common::link_to(&ep).await;
    let watch = common::retrying_client(link.endpoint()).await;
    let told: Arc<Mutex<Vec<Instant>>> = Arc::default();
    let mut _subs = Vec::new();
    for i in 1..=3 {
        let t = Arc::clone(&told);
        _subs.push(
            presence::declare_subscriber(&watch, format!("zk2/lab/s{i}/@zk/alive/**"))
                .history(false)
                .callback(move |s: Sample| {
                    if s.kind() == SampleKind::Put {
                        t.lock().unwrap().push(Instant::now());
                    }
                })
                .await
                .unwrap(),
        );
    }
    // Each open's instant, on a listener of the test's own.
    let (tx, rx) = flume::unbounded::<Instant>();
    let _listener = watch
        .info()
        .transport_events_listener()
        .callback(move |ev| {
            if ev.kind() == SampleKind::Put {
                let _ = tx.send(Instant::now());
            }
        })
        .await
        .unwrap();
    let barrier = presence::Barrier::at(&watch, "zk2/lab/s1/@zk/settle", T)
        .await
        .unwrap();
    assert!(barrier.establish().await, "the round trip at the start");
    let moved = |seen: u64| {
        let b = barrier.clone();
        async move {
            tokio::time::timeout(common::SETTLE, b.moved_since(seen))
                .await
                .expect("the transport moved");
        }
    };
    let mut rows = Vec::new();
    for round in 0..OPEN_ROUNDS {
        let seen = barrier.moves();
        link.cut();
        moved(seen).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let from = told.lock().unwrap().len();
        let seen = barrier.moves();
        link.heal();
        // At once at the open, as the barrier's own waiter hears it.
        moved(seen).await;
        let stood = barrier.establish().await;
        let replied = Instant::now();
        let opened = rx.recv_async().await.unwrap();
        let before = told.lock().unwrap()[from..]
            .iter()
            .filter(|at| **at <= replied)
            .count();
        let after_ms =
            (replied.saturating_duration_since(opened).as_secs_f64() * 1e4).round() / 10.0;
        rows.push((before, after_ms));
        assert!(
            stood,
            "round {round}: the round trip after the reopen stands"
        );
        assert_eq!(
            before, 3,
            "round {round}: the reopen's telling before the barrier's reply"
        );
        // The open's instant, read on two listeners: a few ms between them
        // at most.
        assert!(
            replied + Duration::from_millis(5) >= opened + presence::OPEN_MARGIN,
            "round {round}: asked {after_ms:.3} ms after the open"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    eprintln!(
        "#914: the barrier's round trip after an open stood after the reopen's telling at \
         {} of {OPEN_ROUNDS} reopens; (tokens told by its reply, ms after the open) {rows:?}",
        rows.iter().filter(|(n, _)| *n == 3).count()
    );

    // A move within the margin: the round trip waiting it out ends unasked.
    let seen = barrier.moves();
    link.cut();
    moved(seen).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let asked = presence::liveliness_gets(&watch).settles;
    let seen = barrier.moves();
    link.heal();
    moved(seen).await;
    let opened = Instant::now();
    assert!(
        !barrier.establish_within(Duration::ZERO).await,
        "not asked within the margin"
    );
    assert!(barrier.in_flight(), "waiting out the margin");
    let seen = barrier.moves();
    link.cut();
    moved(seen).await;
    let closed = opened.elapsed();
    eventually("the round trip ends", || async { !barrier.in_flight() }).await;
    assert!(
        !barrier.established(),
        "a move within the margin: no barrier"
    );
    let trips = presence::liveliness_gets(&watch).settles - asked;
    eprintln!("#914: a close {closed:?} after the open; {trips} round trip(s) asked");
    // Under load the close's event can come after the margin, when the
    // round trip asks (through a session with no link, which asks nothing).
    if closed + Duration::from_millis(10) < presence::OPEN_MARGIN {
        assert_eq!(trips, 0, "a move within the margin ends it unasked");
    }
}

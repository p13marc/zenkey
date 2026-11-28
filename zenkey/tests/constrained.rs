//! `spec/scenarios/constrained.md` §2, §3, §5, §7, §8 and §9 (spec §1.6,
//! R7, §3.3, §8.1, §8.5), the parts a Rust runtime can check. §2 and §5
//! name a zenoh-pico owner; here a session without a namespace stands in
//! for it, and the pico itself is spike S15's. §7 counts the bytes that
//! cross toward a far client through a TCP proxy, the link's stand-in, and
//! §9's proxy loses the batch carrying a far client's declaration, or a far
//! router's, as a medium with no ARQ does.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// §3, the GET: the ground's GET through a static binding is answered
/// whenever the link is up, and is silent, not a verdict, while it is cut.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_the_get_is_answered_whenever_the_link_is_up() {
    use zenkey::state::StateGet;
    let (_r1, ep1) = router(None).await;
    let link = common::link_to(&ep1).await;
    let (_r2, ep2) = common::router_via(&link).await;
    let ground = client(&ep1).await;
    let vehicle = client(&ep2).await;
    let nav: IfaceId = "nav.v2".parse().unwrap();

    let mut b = ServiceBuilder::new(&vehicle, config("vehicle-01/nav"));
    b.implement(imp("nav.v2")).unwrap();
    let pose = b
        .declare_state_writer(&nav, "state/pose", &Bindings::new())
        .await
        .unwrap();
    b.expose(&nav, "@op/goto").unwrap();
    b.expose(&nav, "state/tracks/{track}").unwrap();
    let _veh = b.start().await.unwrap();
    pose.put("here").await.unwrap();

    let mut b = ServiceBuilder::new(
        &ground,
        config("ground/ops").bind("nav", &["vehicle-01/nav"]),
    );
    b.require("nav", nav, false);
    let ops = b.start().await.unwrap();
    let consumer = ops
        .consumer("nav", Arc::new(contract("nav.v2")))
        .unwrap()
        .with_presence(Presence::Unavailable);
    let answered = || async {
        matches!(
            consumer.get("state/pose", None, T).await.unwrap(),
            StateGet::Answered(_)
        )
    };
    eventually("answered with the link up", answered).await;
    link.cut();
    eventually("silent while cut", || async { !answered().await }).await;
    link.heal();
    eventually("answered again after the heal", answered).await;
}

/// A far client: a client of the owner's router through a [`common::Link`]
/// of its own, which counts and can copy what crosses toward it.
struct Far {
    link: common::Link,
    session: zenoh::Session,
}

impl Far {
    async fn of(ep: &str) -> Far {
        let link = common::link_to(ep).await;
        let session = client(link.endpoint()).await;
        Far { link, session }
    }

    /// One put on `writer` toward this client, under `sub`: its bytes on
    /// the link, read once it was delivered, so nothing else rides beside
    /// it.
    async fn put(
        &self,
        writer: &zenkey::writer::Writer,
        sub: &zenoh::pubsub::Subscriber<flume::Receiver<Sample>>,
        n: usize,
    ) -> Vec<u8> {
        self.link.capture_down();
        writer.put(format!("{}", n % 10)).await.unwrap();
        tokio::time::timeout(common::SETTLE, sub.recv_async())
            .await
            .expect("a put crossed")
            .unwrap();
        self.link.captured_down()
    }
}

/// What a put toward a far client subscribed to `selector` alone costs on
/// the link: the median over 21 puts, and the bytes of the median put.
async fn per_put(ep: &str, selector: &str, writer: &zenkey::writer::Writer) -> Vec<u8> {
    let far = Far::of(ep).await;
    let sub = far
        .session
        .declare_subscriber(selector)
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    eventually("the far client hears the owner", || async {
        writer.put("0").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        !sub.drain().collect::<Vec<_>>().is_empty()
    })
    .await;
    let mut puts = Vec::new();
    for n in 0..21 {
        puts.push(far.put(writer, &sub, n).await);
    }
    far.session.close().await.unwrap();
    puts.sort_by_key(Vec::len);
    puts.swap_remove(10)
}

/// Whether `text` occurs in `bytes`.
fn carries(bytes: &[u8], text: &str) -> bool {
    bytes.windows(text.len()).any(|w| w == text.as_bytes())
}

/// §7: a far client behind the owner's router subscribes by a member's
/// exact key, by a template's literal prefix, then by `zk2/**`. Each put
/// toward it rides the scope its own subscription declared, and carries the
/// rest of its key as text: none under the exact key, the member under the
/// template's prefix, and everything after `zk2` under `zk2/**`. A reply
/// carries its whole key, whatever the far client declared.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_keys_across_the_face() {
    let (_r1, ep) = router(None).await;
    let owner = client(&ep).await;
    let hop: IfaceId = "hop.v1".parse().unwrap();
    let mut b = ServiceBuilder::new(&owner, config("vehicle-01/hop"));
    b.implement(imp("hop.v1")).unwrap();
    let level = b
        .declare_writer(&hop, "stream/level", &Bindings::new())
        .await
        .unwrap();
    // A member's chunk long enough that finding it in a put's bytes is no
    // chance match of the framing's.
    let roll: Bindings = [("axis".to_owned(), vec!["roll".to_owned()])].into();
    let axis = b
        .declare_writer(&hop, "stream/axis/{axis}", &roll)
        .await
        .unwrap();
    let mode = b
        .declare_state_writer(&hop, "state/mode", &Bindings::new())
        .await
        .unwrap();
    let _svc = b.start().await.unwrap();
    mode.put("auto").await.unwrap();

    let level_key = "zk2/vehicle-01/hop/hop.v1/stream/level";
    let axis_prefix = "zk2/vehicle-01/hop/hop.v1/stream/axis";
    let exact = per_put(&ep, level_key, &level).await;
    let template = per_put(&ep, &format!("{axis_prefix}/*"), &axis).await;
    let wide_level = per_put(&ep, "zk2/**", &level).await;
    let wide_axis = per_put(&ep, "zk2/**", &axis).await;
    // Signed, so a put smaller than expected fails naming both sizes.
    let size = |bytes: &[u8]| i64::try_from(bytes.len()).expect("a put's size");
    let (e, t, w, wa) = (
        size(&exact),
        size(&template),
        size(&wide_level),
        size(&wide_axis),
    );
    eprintln!(
        "constrained.md §7: bytes per put toward the far client: exact key {e}, template \
         prefix {t}, zk2/** {w} (level) and {wa} (axis/roll)"
    );

    // Step 1: the exact key's put carries none of its key's text.
    assert!(!carries(&exact, "stream/level"), "{exact:?}");
    // Step 2: the template's prefix rides a scope; the member is the text.
    assert!(!carries(&template, "stream/axis"), "{template:?}");
    assert!(carries(&template, "/roll"), "{template:?}");
    // Step 3: under zk2/**, every chunk after `zk2` is text.
    let after_zk2 = &level_key["zk2".len()..];
    assert!(carries(&wide_level, after_zk2), "{wide_level:?}");
    let axis_after_zk2 = format!("{}/roll", &axis_prefix["zk2".len()..]);
    assert!(carries(&wide_axis, &axis_after_zk2), "{wide_axis:?}");
    // The puts are alike but for their keys, so the difference is the text
    // and its length's varint, give or take a byte of each link's frame
    // sequence number, a varint from a random start.
    let text = size(after_zk2.as_bytes());
    assert!(
        (text..=text + 3).contains(&(w - e)),
        "zk2/** carries the key's {text} B of text: {w} B against {e} B"
    );
    let member = size(b"/roll");
    assert!(
        (member..=member + 3).contains(&(t - e)),
        "the template's prefix carries /roll: {t} B against {e} B"
    );
    assert!(
        t < wa,
        "{t} B under the template's prefix, {wa} B under zk2/**"
    );

    // Step 4: a reply carries its whole key as text, though the far client
    // declared it exactly.
    let state_key = "zk2/vehicle-01/hop/hop.v1/state/mode";
    let far = Far::of(&ep).await;
    let _sub = far
        .session
        .declare_subscriber(state_key)
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    let reply = common::eventually_some("the owner answers across the face", || async {
        far.link.capture_down();
        let replies = far.session.get(state_key).timeout(T).await.unwrap();
        let mut got = false;
        while let Ok(r) = replies.recv_async().await {
            got |= r.result().is_ok();
        }
        let bytes = far.link.captured_down();
        got.then_some(bytes)
    })
    .await;
    eprintln!(
        "constrained.md §7: a GET's exchange toward the far client: {} B",
        reply.len()
    );
    assert!(carries(&reply, state_key), "{reply:?}");
}

/// A far side across a face of a link.v2 policy, as `FacePolicy::far_side`
/// reads one: `reply_timeout_ms` and `arq` its own, zenoh's transport.
fn far(reply_timeout_ms: u64, arq: bool) -> zenkey::model::link::FarSide {
    zenkey::model::link::FarSide {
        face: "radio".into(),
        reply_timeout_ms,
        arq,
        transport: zenkey::model::link::Binding::Zenoh1_10_1.defaults(),
    }
}

/// §8 (core §8.1, `link.v2` §2.20): the owner's reply takes 1.5 s, behind
/// what a radio queued before it. A caller that waits the contract's 1 s
/// recommendation hears nothing, the router answering `Timeout` and
/// dropping the reply; a caller across the face waits the face's 3 s and
/// gets the value.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s8_a_caller_across_a_face_waits_its_reply_timeout() {
    use zenkey::Client;
    use zenkey::client::Outcome;
    let (_r1, ep) = router(None).await;
    let owner = client(&ep).await;
    let caller = client(&ep).await;
    let key = "zk2/v/slow/faced.v1/@op/slow";
    let queries = owner
        .declare_queryable(key)
        .complete(true)
        .with(flume::unbounded::<zenoh::query::Query>())
        .await
        .unwrap();
    let late = tokio::spawn(async move {
        while let Ok(q) = queries.recv_async().await {
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                q.reply(q.key_expr().clone(), "done").await.unwrap();
            });
        }
    });
    let addr = common::addr("v/slow");
    let contract = Arc::new(contract("faced.v1"));
    let near = Client::new(&caller, Arc::clone(&contract), &["v/slow"])
        .unwrap()
        .with_presence(zenkey::consumer::Presence::Unavailable);
    // The queryable is routed before either call.
    let querier = caller.declare_querier(key).await.unwrap();
    eventually("the owner's queryable is routed", || async {
        querier.matching_status().await.unwrap().matching()
    })
    .await;
    drop(querier);

    // 1. The contract's 1 s: silent, the reply comes after the timeout.
    let t = Instant::now();
    let outcome = near
        .call(&addr, "@op/slow", &Bindings::new(), "go")
        .await
        .unwrap();
    assert!(matches!(outcome, Outcome::NoAnswer(_)), "{outcome:?}");
    assert!(
        t.elapsed() < Duration::from_millis(1400),
        "{:?}",
        t.elapsed()
    );

    // 2. Across the face: at least its 3 s, so the value comes back.
    let across = near.across(&far(3000, false));
    let t = Instant::now();
    let outcome = across
        .call(&addr, "@op/slow", &Bindings::new(), "go")
        .await
        .unwrap();
    let Outcome::Value(answer) = outcome else {
        panic!("across the face the reply comes back: {outcome:?}")
    };
    assert_eq!(answer.payload().to_bytes().as_ref(), b"done");
    assert!(
        t.elapsed() >= Duration::from_millis(1400),
        "{:?}",
        t.elapsed()
    );
    late.abort();
}

/// A stream owner putting `faced.v1`'s `stream/level` (a 1 s horizon)
/// every 100 ms, a client of `ep`; the task that puts.
async fn level_owner(ep: &str) -> (zenkey::Service, tokio::task::JoinHandle<()>) {
    let owner = client(ep).await;
    let faced: IfaceId = "faced.v1".parse().unwrap();
    let mut b = ServiceBuilder::new(&owner, config("v/level"));
    b.implement(imp("faced.v1")).unwrap();
    b.expose(&faced, "@op/slow").unwrap();
    b.expose(&faced, "events/trip").unwrap();
    let level = b
        .declare_writer(&faced, "stream/level", &Bindings::new())
        .await
        .unwrap();
    let svc = b.start().await.unwrap();
    let puts = tokio::spawn(async move {
        loop {
            let _ = level.put("1").await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    (svc, puts)
}

/// How §9 loses a declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lost {
    /// The `DeclareSubscriber`: the key expression's `DeclareKeyExpr`
    /// crossed before, with an earlier subscription the far client made
    /// and undeclared, which the session's runtime keeps for the transport.
    Subscriber,
    /// The batch holding the key's text: its `DeclareKeyExpr`, and the
    /// `DeclareSubscriber` beside it.
    KeyExpr,
}

/// §9 (core §8.5, `link.v2` §2.21): the proxy loses a far consumer's
/// declaration of `stream/level`, its transport left up, and nothing is
/// delivered for two horizons.
/// - **The subscriber's declaration lost**, across a face with no ARQ: the
///   subscription is then declared again, once, and the stream is
///   delivered from then on.
/// - **With ARQ** (the control) it is never declared again, and stays
///   silent.
/// - **The key expression's declaration lost**: declared again, it stays
///   silent, since zenoh 1.10.1 declares a prefix toward its router once a
///   transport (core Appendix B). Two horizons later its repair
///   subscription (`…/level$*`) crosses with a key expression of its own,
///   and the stream is delivered from then on (#1185).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s9_a_lost_declaration_is_declared_again_after_two_silent_horizons() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use zenkey::consumer::Consumer;
    let (_r1, ep) = router(None).await;
    let (_svc, puts) = level_owner(&ep).await;
    let key = "zk2/v/level/faced.v1/stream/level";
    let contract = Arc::new(contract("faced.v1"));
    for (lost, arq) in [
        (Lost::Subscriber, false),
        (Lost::Subscriber, true),
        (Lost::KeyExpr, false),
    ] {
        let case = format!("{lost:?}, arq {arq}");
        let link = common::lossy_link_to(&ep).await;
        let session = client(link.endpoint()).await;
        if lost == Lost::Subscriber {
            // The key expression crosses with a first subscription, which
            // hears the stream and goes.
            let first = session
                .declare_subscriber(key)
                .with(flume::unbounded::<Sample>())
                .await
                .unwrap();
            tokio::time::timeout(common::SETTLE, first.recv_async())
                .await
                .expect("the first subscription hears the stream")
                .unwrap();
            first.undeclare().await.unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
            link.lose_next_up();
        } else {
            link.lose_up(key);
        }
        let consumer = Consumer::for_tool(
            &session,
            Arc::clone(&contract),
            &["v/level"],
            &Default::default(),
        )
        .unwrap()
        .with_presence(Presence::Unavailable)
        .across(&far(1000, arq));
        let heard = Arc::new(AtomicU64::new(0));
        let h = Arc::clone(&heard);
        let t = Instant::now();
        let sub = consumer
            .subscribe("stream/level", move |_| {
                h.fetch_add(1, Ordering::SeqCst);
            })
            .await
            .unwrap();
        eventually("the declaration's batch is lost", || async {
            link.lost_up() == 1
        })
        .await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(heard.load(Ordering::SeqCst), 0, "{case}: nothing crosses");
        match (lost, arq) {
            (Lost::Subscriber, false) => {
                let deadline = Instant::now() + common::SETTLE;
                while heard.load(Ordering::SeqCst) == 0 {
                    assert!(
                        Instant::now() < deadline,
                        "{case}: never crossed, declared again {}",
                        sub.redeclared()
                    );
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                let after = t.elapsed();
                assert!(
                    after >= Duration::from_secs(2),
                    "two horizons first: {after:?}"
                );
                assert_eq!(sub.redeclared(), 1);
                let before = heard.load(Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(1000)).await;
                assert!(heard.load(Ordering::SeqCst) > before, "it goes on crossing");
                assert_eq!(
                    sub.redeclared(),
                    1,
                    "a stream that crosses is not re-declared"
                );
            }
            (Lost::Subscriber, true) => {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                assert_eq!(heard.load(Ordering::SeqCst), 0, "{case}: still silent");
                assert_eq!(sub.redeclared(), 0, "{case}: never declared again");
            }
            (Lost::KeyExpr, _) => {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                assert!(sub.redeclared() >= 1, "{case}: declared again");
                assert_eq!(sub.repaired(), 0, "{case}: not yet repaired");
                assert_eq!(
                    heard.load(Ordering::SeqCst),
                    0,
                    "{case}: a lost DeclareKeyExpr is not declared again on this transport"
                );
                let deadline = Instant::now() + common::SETTLE;
                while heard.load(Ordering::SeqCst) == 0 {
                    assert!(
                        Instant::now() < deadline,
                        "{case}: never repaired, repairs {}",
                        sub.repaired()
                    );
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                let after = t.elapsed();
                assert!(
                    after >= Duration::from_secs(4),
                    "{case}: four horizons first: {after:?}"
                );
                assert_eq!(
                    (sub.redeclared(), sub.repaired()),
                    (1, 1),
                    "{case}: declared again, then repaired once"
                );
            }
        }
        drop(sub);
        session.close().await.unwrap();
    }
    puts.abort();
}

/// A far router's owner for §9 step 5: `faced.v1`'s `stream/level` every
/// 100 ms, its payload a counter, and an event `events/trip` every 200 ms.
async fn level_and_trip_owner(ep: &str) -> (zenkey::Service, tokio::task::JoinHandle<()>) {
    let owner = client(ep).await;
    let faced: IfaceId = "faced.v1".parse().unwrap();
    let mut b = ServiceBuilder::new(&owner, config("v/level"));
    b.implement(imp("faced.v1")).unwrap();
    b.expose(&faced, "@op/slow").unwrap();
    let level = b
        .declare_writer(&faced, "stream/level", &Bindings::new())
        .await
        .unwrap();
    let trip = b
        .event_writer(&faced, "events/trip", &Bindings::new())
        .unwrap();
    let svc = b.start().await.unwrap();
    let puts = tokio::spawn(async move {
        let mut n = 0u64;
        loop {
            let _ = level.put(n.to_string()).await;
            if n.is_multiple_of(2) {
                let _ = trip.put("trip").await;
            }
            n += 1;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    (svc, puts)
}

/// §9 step 5 (core §8.5, #1185): two readers, each a consumer of the
/// stream and of an event, behind one far router, which attaches through
/// the proxy. The far router declared the keys' prefixes with a first
/// subscription, and the proxy drops every batch it sends for 1.5 s while
/// the readers subscribe, the second 0.5 s after the first: their
/// router's `DeclareSubscriber`s.
/// - **Nothing for two horizons**, then each subscription is declared
///   again, the event's beside the stream's, and still nothing: the far
///   router holds the other reader's, and sends nothing.
/// - **At four horizons** the first reader's repairs cross,
///   `…/stream/level$*` and `…/events/trip$*/*`, new at the far router, and
///   both readers hear the stream and the event from then on, each sample
///   once. The second reader, hearing again before its own four horizons,
///   declares no repair.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s9_two_readers_behind_one_far_router_are_repaired_after_four_silent_horizons() {
    use std::sync::Mutex;
    use zenkey::consumer::Consumer;
    let (_ra, ep) = router(None).await;
    let (_svc, puts) = level_and_trip_owner(&ep).await;
    let level = "zk2/v/level/faced.v1/stream/level";
    let trip = "zk2/v/level/faced.v1/events/trip/*";
    let link = common::lossy_link_to(&ep).await;
    let (_rb, ep_b) = common::router_via(&link).await;
    // The far router declares the keys' prefixes toward the near one with
    // a first subscription of each, which hears and goes.
    let primer = client(&ep_b).await;
    for key in [level, trip] {
        let first = primer
            .declare_subscriber(key)
            .with(flume::unbounded::<Sample>())
            .await
            .unwrap();
        tokio::time::timeout(common::SETTLE, first.recv_async())
            .await
            .unwrap_or_else(|_| panic!("the first subscription to {key} hears"))
            .unwrap();
        first.undeclare().await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    link.lose_up_for(Duration::from_millis(1500));
    let contract = Arc::new(contract("faced.v1"));
    type Heard = Arc<Mutex<Vec<(Duration, String)>>>;
    let t = Instant::now();
    let mut readers = Vec::new();
    for i in 0..2 {
        if i == 1 {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let session = client(&ep_b).await;
        let consumer = Consumer::for_tool(
            &session,
            Arc::clone(&contract),
            &["v/level"],
            &Default::default(),
        )
        .unwrap()
        .with_presence(Presence::Unavailable)
        .across(&far(1000, false));
        let (levels, trips): (Heard, Heard) = Default::default();
        let (l, tr) = (Arc::clone(&levels), Arc::clone(&trips));
        let s_level = consumer
            .subscribe("stream/level", move |d| {
                let v = String::from_utf8_lossy(&d.sample.payload().to_bytes()).into_owned();
                l.lock().unwrap().push((t.elapsed(), v));
            })
            .await
            .unwrap();
        let s_trip = consumer
            .subscribe("events/trip", move |d| {
                tr.lock()
                    .unwrap()
                    .push((t.elapsed(), d.sample.key_expr().to_string()));
            })
            .await
            .unwrap();
        readers.push((session, s_level, s_trip, levels, trips));
    }
    tokio::time::sleep(Duration::from_millis(3500)).await;
    assert!(
        link.lost_up() >= 1,
        "the far router's declarations were lost"
    );
    for (_, s_level, s_trip, levels, trips) in &readers {
        assert!(
            levels.lock().unwrap().is_empty() && trips.lock().unwrap().is_empty(),
            "nothing for three horizons and a half"
        );
        assert_eq!(s_level.redeclared(), 1, "declared again once");
        assert_eq!(s_trip.redeclared(), 1, "the event's beside it");
        assert_eq!((s_level.repaired(), s_trip.repaired()), (0, 0));
    }
    eventually("both readers hear the stream and the event", || async {
        readers.iter().all(|(_, _, _, levels, trips)| {
            levels.lock().unwrap().len() >= 10 && !trips.lock().unwrap().is_empty()
        })
    })
    .await;
    for (i, (_, s_level, s_trip, levels, trips)) in readers.iter().enumerate() {
        let levels = levels.lock().unwrap();
        assert!(
            levels[0].0 >= Duration::from_secs(4),
            "four horizons first: {:?}",
            levels[0].0
        );
        assert!(trips.lock().unwrap()[0].0 >= Duration::from_secs(4));
        // Each sample once: the counter only rises.
        let n: Vec<u64> = levels.iter().map(|(_, v)| v.parse().unwrap()).collect();
        assert!(n.windows(2).all(|w| w[0] < w[1]), "each once: {n:?}");
        assert_eq!((s_level.redeclared(), s_trip.redeclared()), (1, 1));
        let repairs = if i == 0 { (1, 1) } else { (0, 0) };
        assert_eq!(
            (s_level.repaired(), s_trip.repaired()),
            repairs,
            "the first reader's repairs serve both"
        );
    }
    for (session, s_level, s_trip, _, _) in readers {
        drop((s_level, s_trip));
        session.close().await.unwrap();
    }
    puts.abort();
}

// --- §10: a first-hop device (#982 H2) ----------------------------------------------

/// The device's session id: the router's block selects its sessions by it,
/// standing in for the far principal's credential (`link.v2` §2.5).
const DEVICE_ZID: &str = "f1f1f1f1f1f1f1f1";

/// A device's session: a client of `endpoint` with the zid `zid`,
/// retrying fast after losing it, as [`common::retrying_client`].
async fn device_session(endpoint: &str, zid: &str) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    for (k, v) in [
        ("scouting/multicast/enabled", "false".to_owned()),
        ("scouting/gossip/enabled", "false".to_owned()),
        ("mode", "\"client\"".to_owned()),
        ("id", format!("\"{zid}\"")),
        ("connect/endpoints", format!("[\"{endpoint}\"]")),
        (
            "connect/retry",
            "{ period_init_ms: 100, period_max_ms: 500, period_increase_factor: 1.5 }".to_owned(),
        ),
    ] {
        c.insert_json5(k, &v).unwrap();
    }
    zenoh::open(c).await.expect("the device's session")
}

/// The near router's block for a device face (`link.v2` §2.5, §2.6), under
/// `allow`: the device's sessions, by zid, meet §2.6's carve, or with
/// `whole` `link.v1` §2.6's whole `zk`, the control.
fn device_block(zid: &str, whole: bool) -> String {
    use zenkey::model::link;
    let denies = if whole {
        vec![serde_json::json!({
            "id": "zk", "flows": link::FLOWS, "messages": link::MESSAGES,
            "key_exprs": [link::ZK_DENY],
        })]
    } else {
        link::zk_carve()
            .iter()
            .map(|d| serde_json::to_value(d).unwrap())
            .collect()
    };
    let rules: Vec<serde_json::Value> = denies
        .into_iter()
        .map(|mut d| {
            d["permission"] = "deny".into();
            d
        })
        .collect();
    let ids: Vec<&serde_json::Value> = rules.iter().map(|r| &r["id"]).collect();
    serde_json::json!({
        "enabled": true,
        "default_permission": "allow",
        "rules": rules,
        "subjects": [{"id": "device", "zids": [zid]}],
        "policies": [{"rules": ids, "subjects": ["device"]}],
    })
    .to_string()
}

/// `link.v2`'s test contract, which uses `freshness.v2`.
fn link_probe() -> zenkey::model::contract::Contract {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spec/profiles/link/conformance/contracts/probe.v1.toml");
    let l = zenkey::model::contract::load_path(&path);
    l.contract.unwrap_or_else(|| panic!("{}", l.report))
}

/// The device: `link.v2`'s probe and `health.v1`, so `freshness.v2` too,
/// each resource exposed, on `config`.
async fn device(
    session: &zenoh::Session,
    config: zenkey::ServiceConfig,
) -> zenkey::Result<zenkey::Service> {
    device_under(session, config, None).await
}

/// [`device`], given `catalog` where one is (core §8.6).
async fn device_under(
    session: &zenoh::Session,
    config: zenkey::ServiceConfig,
    catalog: Option<zenkey::model::catalog::Catalog>,
) -> zenkey::Result<zenkey::Service> {
    let probe = link_probe();
    let iface = probe.iface.clone();
    let mut b = ServiceBuilder::new(session, config);
    if let Some(c) = catalog {
        b.catalog(c)?;
    }
    b.implement(zenkey::Implementation::new(probe))?;
    for res in [
        "stream/frames/link",
        "stream/frames/detail",
        "stream/setpoint",
        "stream/level",
        "state/status",
        "state/config",
    ] {
        b.expose(&iface, res)?;
    }
    b.health()
        .await?
        .set_status(zenkey::health::Level::Ok, "serving")
        .await?;
    b.start().await
}

/// A near holder of `imp`'s bundle (core §8.4): a queryable on its contract
/// key that answers the bundle's bytes.
async fn hold(
    session: &zenoh::Session,
    imp: &zenkey::Implementation,
) -> zenoh::query::Queryable<()> {
    let key = zenkey::model::grammar::contract_key(imp.iface(), imp.fingerprint().hex())
        .unwrap()
        .into_keyexpr();
    let reply = key.clone();
    let bytes = std::sync::Arc::clone(imp.bundle_bytes());
    session
        .declare_queryable(key)
        .complete(true)
        .callback(move |q| {
            use zenoh::Wait;
            let _ = q.reply(reply.clone(), bytes.to_vec()).wait();
        })
        .await
        .unwrap()
}

/// What a device declared toward its router, read from the bytes it sent:
/// whether its instance key, an interface token or a contract key crossed.
/// Each declaration carries its key as text (core Appendix B).
fn declared(up: &[u8]) -> (bool, Vec<&'static str>, bool) {
    let alive = ["probe.v1", "health.v1", "freshness.v2"]
        .into_iter()
        .filter(|i| carries(up, &format!("lab/probe-a/@zk/alive/{i}/")))
        .collect();
    (
        carries(up, "lab/probe-a/@zk/instance/"),
        alive,
        carries(up, "@zk/contract/"),
    )
}

/// §10: a device owner on a first hop holds its instance token alone (core
/// §8.1, §8.2 step 3, §8.4), and the near side finds it by that token and
/// its descriptor (observer §3.6), through a device face's carve (`link.v2`
/// §2.6). Steps 1, 2, 3, 4, 5 and 6; what a tool reports of the closed link
/// is the fleet's (H3).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s10_a_first_hop_device() {
    use zenkey::consumer::Consumer;
    let (_r1, ep) = common::router_with(
        None,
        &[("access_control", &device_block(DEVICE_ZID, false))],
    )
    .await;
    let link = common::link_to(&ep).await;
    let tool = client(&ep).await;
    let holder = client(&ep).await;
    let probe = zenkey::Implementation::new(link_probe());
    let _held = hold(&holder, &probe).await;
    let at = common::addr("lab/probe-a");

    // 1. The device starts, configured for a first hop.
    link.capture_up();
    let session = device_session(link.endpoint(), DEVICE_ZID).await;
    let svc = device(&session, config("lab/probe-a").first_hop())
        .await
        .expect("a first-hop device starts");
    let key = svc.instance_key().unwrap().to_string();
    eventually("the instance token is in", || async {
        presence::liveliness_keys(&tool, "zk2/lab/probe-a/@zk/**", T)
            .await
            .unwrap()
            == [key.clone()]
    })
    .await;
    let d = presence::descriptor(&tool, &at, svc.instance(), T)
        .await
        .unwrap()
        .into_descriptor()
        .expect("the descriptor crosses");
    let mut listed: Vec<(&str, bool)> = d
        .interfaces
        .iter()
        .map(|e| (e.iface.as_str(), e.token))
        .collect();
    listed.sort_unstable();
    assert_eq!(
        listed,
        [
            ("freshness.v2", false),
            ("health.v1", false),
            ("probe.v1", false)
        ],
        "every interface listed, none with a token"
    );
    let consumer = Consumer::for_tool(
        &tool,
        Arc::new(link_probe()),
        &["lab/probe-a"],
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        consumer.wait_for_provider(common::SETTLE).await.unwrap(),
        Some(at.clone()),
        "the consumer's wait ends on the instance token and the descriptor"
    );
    let got = zenkey::retrieval::fetch_bundle(&tool, probe.iface(), probe.fingerprint(), T)
        .await
        .unwrap();
    assert!(
        matches!(got, zenkey::retrieval::Retrieved::Bundle(..)),
        "the bundle is held near: {got:?}"
    );
    assert_eq!(
        declared(&link.captured_up()),
        (true, Vec::new(), false),
        "toward the router: the instance key, no interface token, no contract queryable"
    );

    // 2. A new transport: the same declarations again, and nothing more.
    link.cut();
    eventually("the token goes with the link", || async {
        presence::liveliness_keys(&tool, "zk2/lab/probe-a/@zk/**", T)
            .await
            .unwrap()
            .is_empty()
    })
    .await;
    link.capture_up();
    link.heal();
    eventually("the token is back", || async {
        presence::liveliness_keys(&tool, "zk2/lab/probe-a/@zk/**", T)
            .await
            .unwrap()
            == [key.clone()]
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        declared(&link.captured_up()),
        (true, Vec::new(), false),
        "the new transport declares step 1's again"
    );

    // 3. A new run: the tool hears the descriptor that states it.
    let puts = tool
        .declare_subscriber(key.as_str())
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let run = svc.new_run();
    let heard = tokio::time::timeout(common::SETTLE, async {
        loop {
            let s = puts.recv_async().await.unwrap();
            let v: serde_json::Value =
                serde_json::from_slice(&s.payload().to_bytes()).expect("a descriptor");
            if v["run"] == run.hex() {
                break v;
            }
        }
    })
    .await
    .expect("the new run's descriptor crosses");
    assert_eq!(heard["instance"], svc.instance().as_str());
    drop(svc);
    session.close().await.unwrap();

    // 5. The control: a device at its defaults, under step 1's block. Every
    // declaration crosses, and the face denies all but the instance token.
    link.capture_up();
    let session = device_session(link.endpoint(), DEVICE_ZID).await;
    let svc = device(&session, config("lab/probe-a")).await.unwrap();
    let key = svc.instance_key().unwrap().to_string();
    eventually("the instance token alone is seen", || async {
        presence::liveliness_keys(&tool, "zk2/lab/probe-a/@zk/**", T)
            .await
            .unwrap()
            == [key.clone()]
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        declared(&link.captured_up()),
        (true, vec!["probe.v1", "health.v1", "freshness.v2"], true),
        "at its defaults every token and contract queryable crosses"
    );
    drop(svc);
    session.close().await.unwrap();

    // 6. A first-hop device that implements a governed interface does not
    // start, and no instance token crosses (§8.1, §8.2 step 2).
    link.capture_up();
    let session = device_session(link.endpoint(), DEVICE_ZID).await;
    let mut b = ServiceBuilder::new(&session, config("lab/probe-a").first_hop());
    b.implement(imp("steer-governed.v1")).unwrap();
    let Err(refused) = b.start().await else {
        panic!("refused at step 2")
    };
    assert!(
        refused.to_string().contains("first hop"),
        "refused for the first hop: {refused}"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!declared(&link.captured_up()).0, "no instance token");
    session.close().await.unwrap();
}

/// The device's three revisions: `probe.v1`, `health.v1` and
/// `freshness.v2`, as the device implements them.
fn device_revisions() -> [zenkey::Implementation; 3] {
    [
        zenkey::Implementation::new(link_probe()),
        zenkey::health::implementation(),
        zenkey::heartbeat::implementation(),
    ]
}

/// The deployment catalog of the device's three revisions (core §8.6).
fn device_catalog() -> zenkey::model::catalog::Catalog {
    let mut c = zenkey::model::catalog::Catalog::new(0).unwrap();
    for imp in device_revisions() {
        c.push(imp.iface().clone(), imp.fingerprint().clone())
            .unwrap();
    }
    c
}

/// What one descriptor put of `svc` costs across the hop, toward its
/// router: the bytes `link` carries up while `svc` takes a new run, whose
/// descriptor `heard` hears put, the median of 9. Each round is read once
/// the put was heard, so little else rides beside it.
async fn descriptor_put(
    link: &common::Link,
    svc: &zenkey::Service,
    heard: &zenoh::pubsub::Subscriber<flume::Receiver<Sample>>,
) -> (usize, usize) {
    let mut rounds = Vec::new();
    for _ in 0..9 {
        let _ = heard.drain().count();
        link.capture_up();
        svc.new_run();
        let s = tokio::time::timeout(common::SETTLE, heard.recv_async())
            .await
            .expect("the descriptor put crosses")
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        rounds.push((link.captured_up().len(), s.payload().len()));
    }
    rounds.sort_unstable();
    rounds[4]
}

/// §10 step 7 (#980 C5c): a device on a first hop given the deployment
/// catalog writes its descriptor in the compact form (core §3.3, §8.1),
/// under `zenoh/bytes`, which crosses the hop in a few tens of bytes where
/// the JSON form takes some hundreds. The tool holding the catalog reads it
/// to the members the JSON form states, retrieving the revisions it names
/// from the near holder (§8.4), and the consumer holding it finds the
/// device (observer §3.6). Holding none, neither reads it: the tool's read
/// is `unknown_catalog`, and the consumer's wait does not end, the device
/// unobservable, never absent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s10_a_first_hop_device_writes_its_descriptor_compact() {
    use zenkey::consumer::Consumer;
    use zenkey::model::descriptor::{Form, compact};
    let (_r1, ep) = common::router_with(
        None,
        &[("access_control", &device_block(DEVICE_ZID, false))],
    )
    .await;
    let link = common::link_to(&ep).await;
    let tool = client(&ep).await;
    let holder = client(&ep).await;
    let mut held = Vec::new();
    for imp in device_revisions() {
        held.push(hold(&holder, &imp).await);
    }
    let at = common::addr("lab/probe-a");
    let catalog = device_catalog();
    let heard = tool
        .declare_subscriber("zk2/lab/probe-a/@zk/instance/*")
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();

    // The JSON form first: a first hop given no catalog.
    let session = device_session(link.endpoint(), DEVICE_ZID).await;
    let svc = device(&session, config("lab/probe-a").first_hop())
        .await
        .unwrap();
    assert_eq!(svc.descriptor_form(), Form::Json, "no catalog: JSON");
    eventually("the JSON device is heard", || async {
        heard.drain().count() > 0 || {
            svc.new_run();
            false
        }
    })
    .await;
    let (json_up, json_payload) = descriptor_put(&link, &svc, &heard).await;
    let json = presence::descriptor(&tool, &at, svc.instance(), T)
        .await
        .unwrap()
        .into_descriptor()
        .expect("the JSON descriptor reads with nothing held");
    drop(svc);
    session.close().await.unwrap();

    // 7. The same device given the catalog: the compact form, by default.
    let session = device_session(link.endpoint(), DEVICE_ZID).await;
    let svc = device_under(
        &session,
        config("lab/probe-a").first_hop(),
        Some(catalog.clone()),
    )
    .await
    .expect("a first-hop device given a catalog starts");
    assert_eq!(svc.descriptor_form(), Form::Compact, "§8.1's SHOULD");
    let key = svc.instance_key().unwrap().to_string();
    eventually("the compact device is heard", || async {
        heard.drain().count() > 0 || {
            svc.new_run();
            false
        }
    })
    .await;
    let (compact_up, compact_payload) = descriptor_put(&link, &svc, &heard).await;
    eprintln!(
        "constrained.md §10 step 7: the descriptor's put across the hop, JSON {json_up} B \
         (payload {json_payload} B), compact {compact_up} B (payload {compact_payload} B)"
    );
    assert!(
        compact_payload < 64 && compact_up < 160,
        "the compact form's put: {compact_up} B, its payload {compact_payload} B"
    );
    assert!(
        json_payload > 400 && compact_up * 4 < json_up,
        "JSON {json_up} B against compact {compact_up} B"
    );

    // Its put and its GET's reply are under `zenoh/bytes`, the envelope's run
    // the one the instance writes under. (The router stamps what it
    // forwards, so a subscriber's sample carries the router's timestamp.)
    let s = tokio::time::timeout(common::SETTLE, async {
        loop {
            let _ = heard.drain().count();
            let run = svc.new_run();
            let s = heard.recv_async().await.unwrap();
            if compact::Head::parse(&s.payload().to_bytes()).is_ok_and(|h| h.stamp.run == run) {
                break s;
            }
        }
    })
    .await
    .expect("a put stating the new run");
    assert_eq!(s.encoding().to_string(), "zenoh/bytes");
    let reply = presence::descriptor_reply(&tool, &at, svc.instance(), T)
        .await
        .unwrap()
        .expect("the GET is answered");
    assert_eq!(reply.form, Form::Compact);
    assert_eq!(&*reply.bytes, &*svc.descriptor_bytes());

    // The tool holding the catalog reads it to the JSON form's members,
    // retrieving the three revisions from the near holder.
    let through = presence::Held::new(Arc::new(vec![catalog.clone()]), Vec::new()).retrieving();
    let d = presence::descriptor_held(&tool, &at, svc.instance(), &through, T)
        .await
        .unwrap()
        .into_descriptor()
        .expect("read through the catalog");
    let pairs = |d: &zenkey::model::descriptor::Descriptor| {
        let mut p: Vec<(String, String, bool)> = d
            .interfaces
            .iter()
            .map(|e| (e.iface.clone(), e.contract.clone(), e.token))
            .collect();
        p.sort_unstable();
        p
    };
    assert_eq!(pairs(&d), pairs(&json), "the members the JSON form states");
    assert_eq!(d.capabilities, json.capabilities);
    assert_eq!(d.catalog, Some(catalog.id().hex()));
    assert_eq!(d.meta.get("zid"), json.meta.get("zid"), "the zid stays");
    // Holding none: not read, never guessed.
    match presence::descriptor(&tool, &at, svc.instance(), T)
        .await
        .unwrap()
    {
        Found::Unread(why) => assert!(why.starts_with("unknown_catalog"), "{why}"),
        other => panic!("not read without the catalog: {other:?}"),
    }

    // The consumer holding the catalog finds the device by its instance
    // token and its descriptor; holding none, its wait does not end.
    let consumer = |catalog: Option<&zenkey::model::catalog::Catalog>| {
        let c = Consumer::for_tool(
            &tool,
            Arc::new(link_probe()),
            &["lab/probe-a"],
            &Default::default(),
        )
        .unwrap();
        match catalog {
            Some(cat) => c.with_catalog(cat),
            None => c,
        }
    };
    assert_eq!(
        consumer(Some(&catalog))
            .wait_for_provider(common::SETTLE)
            .await
            .unwrap(),
        Some(at.clone()),
        "found through the catalog"
    );
    assert_eq!(
        consumer(None)
            .wait_for_provider(Duration::from_secs(3))
            .await
            .unwrap(),
        None,
        "not found without it, and never concluded absent"
    );
    assert_eq!(
        presence::liveliness_keys(&tool, "zk2/lab/probe-a/@zk/**", T)
            .await
            .unwrap(),
        [key],
        "its instance token alone crosses"
    );
    drop(held);
}

/// Core §3.3, §8.1 (#980 C5c): the compact form asked for needs a catalog
/// and no role, each refused at start, nothing declared; asked off, a
/// first hop given a catalog writes JSON.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_compact_form_asked_for_needs_a_catalog_and_no_role() {
    use zenkey::model::descriptor::Form;
    let (_r1, ep) = router(None).await;
    let session = client(&ep).await;
    let Err(e) = device(&session, config("lab/a").compact_descriptor(true)).await else {
        panic!("refused without a catalog");
    };
    assert!(e.to_string().contains("no catalog"), "{e}");
    let mut b = ServiceBuilder::new(
        &session,
        config("lab/b")
            .compact_descriptor(true)
            .bind("p", &["lab/x"]),
    );
    b.catalog(device_catalog()).unwrap();
    b.require("p", "probe.v1".parse().unwrap(), false);
    let Err(e) = b.start().await else {
        panic!("refused beside a role");
    };
    assert!(e.to_string().contains("role"), "{e}");
    let svc = device_under(
        &session,
        config("lab/c").first_hop().compact_descriptor(false),
        Some(device_catalog()),
    )
    .await
    .unwrap();
    assert_eq!(svc.descriptor_form(), Form::Json, "asked off");
    // Off a first hop, the MAY: the compact form when asked.
    let svc = device_under(
        &session,
        config("lab/d").compact_descriptor(true),
        Some(device_catalog()),
    )
    .await
    .unwrap();
    assert_eq!(svc.descriptor_form(), Form::Compact);
    let svc_json = device_under(&session, config("lab/e"), Some(device_catalog()))
        .await
        .unwrap();
    assert_eq!(
        svc_json.descriptor_form(),
        Form::Json,
        "the near side's default"
    );
    drop(svc);
}

/// What `start` refuses of a first-hop owner implementing `imps`, nothing
/// exposed.
async fn first_hop_refusal(session: &zenoh::Session, imps: Vec<zenkey::Implementation>) -> String {
    let mut b = ServiceBuilder::new(session, config("lab/probe-a").first_hop());
    for i in imps {
        b.implement(i).unwrap();
    }
    match b.start().await {
        Err(refused) => refused.to_string(),
        Ok(_) => panic!("started: refused at step 2"),
    }
}

/// Core §8.1 on a first hop refuses first (#982 H2; review-bot on #1068):
/// `archive.v1` or a governed interface is refused before any other check
/// of §8.2 step 2, so the refusal names the first hop. Nothing is exposed
/// here, which exposure would refuse, and `archive.v1` would join the
/// tokenless set, which the archive's own check would refuse.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s10_a_first_hop_refuses_before_any_other_step_2_check() {
    let (_r1, ep) = router(None).await;
    let session = client(&ep).await;
    let governed =
        first_hop_refusal(&session, vec![imp("ping.v1"), imp("steer-governed.v1")]).await;
    // `steer-governed.v1.toml` is steer.v1 at minor 1, governed: the
    // refusal names the interface, never the file.
    assert!(
        governed.contains("steer.v1 is refused on a first hop")
            && governed.contains("redundancy.v1"),
        "the first hop's refusal, before exposure's: {governed}"
    );
    let archive = first_hop_refusal(
        &session,
        vec![zenkey::Implementation::new(zenkey::archive::contract())],
    )
    .await;
    assert!(
        archive.contains("archive.v1 is refused on a first hop"),
        "the first hop's refusal, before exposure's and the tokenless set's: {archive}"
    );
    assert!(!archive.contains("never tokenless"), "{archive}");
}

/// §10 step 4, the control: a block that keeps `link.v1` §2.6's whole `zk`
/// on the device's face. The near side lists no instance of it, its
/// presence unobservable there, and still receives its heartbeats (R7).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s10_the_whole_zk_hides_a_first_hop_device() {
    let (_r1, ep) =
        common::router_with(None, &[("access_control", &device_block(DEVICE_ZID, true))]).await;
    let link = common::link_to(&ep).await;
    let tool = client(&ep).await;
    let beats = tool
        .declare_subscriber("zk2/lab/probe-a/freshness.v2/stream/heartbeat/**")
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let session = device_session(link.endpoint(), DEVICE_ZID).await;
    let _svc = device(&session, config("lab/probe-a").first_hop())
        .await
        .unwrap();
    tokio::time::timeout(common::SETTLE, beats.recv_async())
        .await
        .expect("a heartbeat crosses")
        .unwrap();
    assert!(
        presence::liveliness_keys(&tool, "zk2/lab/probe-a/@zk/**", T)
            .await
            .unwrap()
            .is_empty(),
        "no instance of it is listed"
    );
}

//! `spec/scenarios/constrained.md` §2, §3, §5 and §7 (spec §1.6, R7, §3.3,
//! §8.5), the parts a Rust runtime can check. §2 and §5 name a zenoh-pico
//! owner; here a session without a namespace stands in for it, and the pico
//! itself is spike S15's. §7 counts the bytes that cross toward a far client
//! through a TCP proxy, the link's stand-in.

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

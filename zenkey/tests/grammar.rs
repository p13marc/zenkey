//! `spec/scenarios/grammar.md` §1–§2 (spec §1.3 the guard, §1.6
//! namespaces).

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::{T, client, client_ns, config, eventually, imp, router};
use zenkey::model::grammar::IfaceId;
use zenkey::model::template::Bindings;
use zenkey::presence;
use zenkey::{Service, ServiceBuilder};
use zenoh::Wait;
use zenoh::pubsub::Publisher;
use zenoh::query::{ConsolidationMode, QueryTarget, Queryable};
use zenoh::sample::Sample;

/// A lowercase ULID chunk (§1.2), as an event's last chunk (§2.6).
const ULID: &str = "01j0000000000000000000000a";

fn probe() -> IfaceId {
    "probe.v1".parse().unwrap()
}

/// The owner of grammar.md: one resource of every kind token, and which of
/// its queryables a GET reached.
struct Owner {
    svc: Service,
    publishers: Vec<(String, Publisher<'static>)>,
    event_key: String,
    hits: Vec<(&'static str, Arc<AtomicBool>)>,
    _queryables: Vec<Queryable<()>>,
}

async fn owner(s: &zenoh::Session, address: &str) -> Owner {
    let mut b = ServiceBuilder::new(s, config(address));
    b.implement(imp("probe.v1")).unwrap();
    let none = Bindings::new();
    let mut publishers = Vec::new();
    for res in ["stream/s", "@stream/xs", "state/st", "@state/xst"] {
        publishers.push((
            res.to_owned(),
            b.declare_publisher(&probe(), res, &none).await.unwrap(),
        ));
    }
    let mut hits = Vec::new();
    let mut queryables = Vec::new();
    for (res, name) in [
        ("state/st", "state"),
        ("@state/xst", "@state"),
        ("@op/op", "@op"),
    ] {
        let hit = Arc::new(AtomicBool::new(false));
        let h = Arc::clone(&hit);
        let key = b.key(&probe(), res, &none).unwrap().into_keyexpr();
        queryables.push(
            b.declare_queryable(&probe(), res, Some(&none), move |q| {
                h.store(true, Ordering::SeqCst);
                let _ = q.reply(key.clone(), "v").wait();
            })
            .await
            .unwrap(),
        );
        hits.push((name, hit));
    }
    b.expose(&probe(), "events/ev").unwrap();
    let event_key = format!("{}/{ULID}", b.key(&probe(), "events/ev", &none).unwrap());
    Owner {
        svc: b.start().await.unwrap(),
        publishers,
        event_key,
        hits,
        _queryables: queryables,
    }
}

impl Owner {
    /// Puts on every data resource once.
    async fn put_all(&self, s: &zenoh::Session) {
        for (_, p) in &self.publishers {
            p.put("v").await.unwrap();
        }
        s.put(self.event_key.as_str(), "v").await.unwrap();
    }

    fn hit(&self, name: &str) -> bool {
        self.hits
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap()
            .1
            .load(Ordering::SeqCst)
    }
}

/// The kind token (position 5) of each key, as a set.
fn kinds(keys: &[String], base: usize) -> BTreeSet<String> {
    keys.iter()
        .map(|k| k.split('/').nth(base + 4).unwrap().to_owned())
        .collect()
}

/// Subscribes to `sel` and puts on every data resource until each kind in
/// `expect` has arrived, then drains a little longer. A matching check is
/// not enough here: a subscriber dropped by the previous step can still
/// satisfy it while its undeclaration is in flight.
async fn delivered(
    owner: &Owner,
    owner_s: &zenoh::Session,
    sub_s: &zenoh::Session,
    sel: &str,
    expect: &[&str],
    base: usize,
) -> Vec<String> {
    let sub = sub_s
        .declare_subscriber(sel)
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    let want: BTreeSet<String> = expect.iter().map(|k| (*k).to_owned()).collect();
    let mut keys = BTreeSet::new();
    let deadline = tokio::time::Instant::now() + common::SETTLE;
    while !kinds(&keys.iter().cloned().collect::<Vec<_>>(), base).is_superset(&want) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "only {keys:?} on {sel}"
        );
        owner.put_all(owner_s).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        keys.extend(sub.drain().map(|s| s.key_expr().as_str().to_owned()));
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    keys.extend(sub.drain().map(|s| s.key_expr().as_str().to_owned()));
    keys.into_iter().collect()
}

async fn get_keys(s: &zenoh::Session, sel: &str) -> Vec<String> {
    let rx = s
        .get(sel)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(T)
        .with(flume::unbounded())
        .await
        .unwrap();
    let mut keys = Vec::new();
    while let Ok(r) = rx.recv_async().await {
        if let Ok(s) = r.result() {
            keys.push(s.key_expr().as_str().to_owned());
        }
    }
    keys.sort();
    keys
}

/// §1: ambient selectors never deliver `@stream`/`@state`, never reach an
/// `@op` or control queryable, and exact kind selectors select exactly.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_the_guard() {
    let (_r1, ep) = router(None).await;
    let owner_s = client(&ep).await;
    let consumer = client(&ep).await;
    let o = owner(&owner_s, "g/svc").await;
    let inst = o.svc.instance_key().unwrap().to_string();
    eventually("the owner is alive", || async {
        presence::liveliness_keys(&consumer, &inst, T)
            .await
            .unwrap()
            == [inst.clone()]
    })
    .await;

    // 1. zk2/g/** delivers stream, state and events only.
    let got = delivered(
        &o,
        &owner_s,
        &consumer,
        "zk2/g/**",
        &["events", "state", "stream"],
        0,
    )
    .await;
    assert_eq!(
        kinds(&got, 0),
        BTreeSet::from(["events", "state", "stream"].map(String::from)),
        "{got:?}"
    );

    // 2. A GET on zk2/g/** reaches the plain state queryable only.
    let got = get_keys(&consumer, "zk2/g/**").await;
    assert_eq!(got, ["zk2/g/svc/probe.v1/state/st"]);
    assert!(o.hit("state") && !o.hit("@state") && !o.hit("@op"));

    // 3. Liveliness: nothing ambient; the instance token under @zk.
    assert!(
        presence::liveliness_keys(&consumer, "zk2/g/**", T)
            .await
            .unwrap()
            .is_empty()
    );
    let zk = presence::liveliness_keys(&consumer, "zk2/g/*/@zk/**", T)
        .await
        .unwrap();
    assert!(zk.contains(&inst), "{zk:?}");

    // 4. Exact kind selectors: plain state only.
    let got = delivered(&o, &owner_s, &consumer, "zk2/*/*/*/state/**", &["state"], 0).await;
    assert_eq!(got, ["zk2/g/svc/probe.v1/state/st"]);
    assert_eq!(
        get_keys(&consumer, "zk2/*/*/*/state/**").await,
        ["zk2/g/svc/probe.v1/state/st"]
    );
}

/// §2: the namespace is added on egress and stripped on ingress; other
/// namespaces see nothing; the guard holds under a prefix.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_namespaces() {
    let (_r1, ep) = router(None).await;
    let owner_s = client_ns(&ep, Some("dep1")).await;
    let same = client_ns(&ep, Some("dep1")).await;
    let bare = client(&ep).await;
    let other = client_ns(&ep, Some("dep2")).await;
    let o = owner(&owner_s, "g/svc").await;
    let inst = o.svc.instance_key().unwrap().to_string();
    eventually("the owner is alive in dep1", || async {
        presence::liveliness_keys(&same, &inst, T).await.unwrap() == [inst.clone()]
    })
    .await;

    // dep1 sees base-relative keys.
    let got = delivered(
        &o,
        &owner_s,
        &same,
        "zk2/**",
        &["events", "state", "stream"],
        0,
    )
    .await;
    assert!(
        !got.is_empty() && got.iter().all(|k| k.starts_with("zk2/g/svc/")),
        "{got:?}"
    );
    // No namespace: the prefixed keys; and the guard holds under it.
    let got = delivered(
        &o,
        &owner_s,
        &bare,
        "dep1/zk2/**",
        &["events", "state", "stream"],
        1,
    )
    .await;
    assert_eq!(
        kinds(&got, 1),
        BTreeSet::from(["events", "state", "stream"].map(String::from)),
        "{got:?}"
    );
    assert_eq!(
        get_keys(&bare, "dep1/zk2/**").await,
        ["dep1/zk2/g/svc/probe.v1/state/st"]
    );
    assert!(!o.hit("@state") && !o.hit("@op"));
    // dep2 sees nothing.
    let sub = other
        .declare_subscriber("zk2/**")
        .with(flume::unbounded::<Sample>())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    o.put_all(&owner_s).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(sub.drain().count(), 0);
    assert!(get_keys(&other, "zk2/**").await.is_empty());
}

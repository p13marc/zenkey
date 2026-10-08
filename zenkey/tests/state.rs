//! `spec/scenarios/state.md` (spec §4, §2.6). §8 (events replay) is the
//! streams chunk's (#619); the state rules S1–S7 join it with #620.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use common::{T, client, config, contract, eventually, imp, router};
use zenkey::ServiceBuilder;
use zenkey::model::grammar::IfaceId;
use zenkey::model::template::Bindings;
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

//! The schema store's negative cache, against real zenoh (issue #101).
//!
//! `SchemaStore` used to collapse two very different outcomes into one 60s
//! verdict: "somebody answered and served no `describe`" and "nobody answered
//! at all". The first deserves the bound — a producer that serves no schemas
//! must not be re-asked per sample. The second is the RFC 05 §3.1 non-verdict,
//! and it is exactly what an explorer started *before* its fleet sees: for a
//! minute, protobuf and CDR leaves render structurally and the honest "no
//! schema served" line is wrong.
//!
//! That is not hypothetical — it is what made `codec.rs` flaky, and why that
//! file still proves its fixture routable with a raw GET before building a
//! store. These tests pin both halves of the fix.
//!
//! Self-contained: two in-process peers, explicit endpoints, no scouting, no
//! external router. Ports 7507-7508 (disjoint from every other test binary).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use zenkey::schema::{SchemaSet, TypeSchema};
use zenkey_fleet::decode::SchemaStore;

const ORIGIN: &str = "h-bbbbbbbbbbbb";
const PRODUCER: &str = "sysinfo";

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

fn schema_set_json() -> String {
    SchemaSet::builder("t")
        .entry(
            "Point",
            TypeSchema::json_schema(serde_json::json!({
                "type": "object",
                "properties": {"x": {"type": "integer"}},
            })),
        )
        .build()
        .to_json()
}

/// A `describe` queryable that counts how many times it was asked.
async fn declare_describe(
    session: &zenoh::Session,
    payload: String,
) -> (zenoh::query::Queryable<()>, Arc<AtomicUsize>) {
    let asked = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&asked);
    let key = format!("v1/{ORIGIN}/@rpc/{PRODUCER}/describe");
    let reply_key = key.clone();
    let queryable = session
        .declare_queryable(&key)
        .callback(move |query| {
            counter.fetch_add(1, Ordering::Relaxed);
            let q = query.clone();
            let reply_key = reply_key.clone();
            let payload = payload.clone();
            tokio::spawn(async move {
                q.reply(reply_key, payload).await.unwrap();
            });
        })
        .await
        .expect("queryable");
    (queryable, asked)
}

/// Wait until a raw GET on the describe key draws a reply.
async fn wait_routable(session: &zenoh::Session) {
    let key = zenkey::selector::fleet_rpc(PRODUCER, &["describe"]).to_string();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let answers = zenkey_fleet::fleet_get(session, "", &key, None, Duration::from_secs(1))
                .await
                .unwrap_or_default();
            if !answers.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the fixture should be routable within 10s");
}

/// The headline: a store that asked too early must recover in a bounded
/// retry, not sit blind for the full TTL.
///
/// The first ask happens while **nothing** serves describe, which is what
/// guarantees the zero-reply path is the one under test. Before this fix that
/// first `None` was authoritative for 60 seconds, so the assertion below could
/// not have been met by waiting — only by throwing the store away.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_store_that_asked_too_early_recovers_without_waiting_out_the_ttl() {
    let (listen, connect) = peer_pair(7507).await;
    let store = SchemaStore::new("", Duration::from_secs(2));

    // Ask before anyone serves: zero replies, and not a verdict.
    assert!(
        store.set_for(&connect, PRODUCER).await.is_none(),
        "nothing serves describe yet"
    );

    // The fleet arrives.
    let (_q, asked) = declare_describe(&listen, schema_set_json()).await;
    wait_routable(&connect).await;

    let started = Instant::now();
    let resolved = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Some(set) = store.set_for(&connect, PRODUCER).await {
                return set;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the store must re-ask long before the 60s TTL");

    assert!(resolved.get("Point").is_some(), "the served type resolves");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "recovery took {:?}",
        started.elapsed()
    );
    assert!(
        asked.load(Ordering::Relaxed) > 0,
        "the store must actually have re-asked the producer"
    );
}

/// The bound that must **not** be lost: a producer that answers, and serves
/// nothing a store can use, is a verdict about that producer — asked at most
/// once per TTL, however many samples arrive.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_producer_that_answers_with_junk_is_asked_at_most_once_per_ttl() {
    let (listen, connect) = peer_pair(7508).await;
    let (_q, asked) = declare_describe(&listen, "this is not a SchemaSet".to_string()).await;
    wait_routable(&connect).await;

    let store = SchemaStore::new("", Duration::from_secs(2));
    let before = asked.load(Ordering::Relaxed);

    for _ in 0..25 {
        assert!(
            store.set_for(&connect, PRODUCER).await.is_none(),
            "junk is not a schema set"
        );
    }
    assert_eq!(
        asked.load(Ordering::Relaxed) - before,
        1,
        "an answered-but-unusable producer must be asked once per TTL, not per call"
    );

    // …and the explicit escape hatch still works, because a frontend has to
    // be able to say "ask again" without restarting.
    store.forget(PRODUCER);
    assert!(store.set_for(&connect, PRODUCER).await.is_none());
    assert_eq!(
        asked.load(Ordering::Relaxed) - before,
        2,
        "forget() must send the next question to the bus"
    );
}

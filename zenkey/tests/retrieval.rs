//! `spec/scenarios/retrieval.md` §1–§3 (spec §8.4).
//!
//! Common setup: holders of the `camera.v1` bundle. The nearest holder sits
//! on the client's router (R1), and others behind a second router (R2 → R1).

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{SETTLE, T, client, imp, router};
use zenkey::Implementation;
use zenkey::model::grammar::contract_key;
use zenkey::retrieval::{Retrieved, fetch_bundle};
use zenoh::Wait;
use zenoh::query::{ConsolidationMode, Query, QueryTarget, Queryable};

#[derive(Clone, Copy)]
enum Mode {
    Good,
    /// Replies after this long.
    Slow(Duration),
    /// Never replies, and never finalizes: a frozen process.
    Frozen,
    /// Replies with a corrupted bundle.
    Corrupt,
}

struct Holder {
    _session: zenoh::Session,
    _q: Queryable<()>,
    _frozen: Arc<Mutex<Vec<Query>>>,
}

fn camera() -> Implementation {
    imp("camera.v1")
}

async fn holder(endpoint: &str, mode: Mode) -> Holder {
    let imp = camera();
    let session = client(endpoint).await;
    let key = contract_key(imp.iface(), imp.fingerprint().hex())
        .unwrap()
        .into_keyexpr();
    let mut bytes = imp.bundle_bytes().to_vec();
    if let Mode::Corrupt = mode {
        // A schema-free contract: corrupt the contract itself, so the
        // fingerprint check (or an earlier one) refuses it.
        let at = bytes.iter().position(|&b| b == b'c').unwrap();
        bytes[at] = b'k';
    }
    let frozen: Arc<Mutex<Vec<Query>>> = Arc::default();
    let f = Arc::clone(&frozen);
    let reply_key = key.clone();
    let q = session
        .declare_queryable(key)
        .complete(true)
        .callback(move |q| match mode {
            Mode::Good | Mode::Corrupt => {
                let _ = q.reply(reply_key.clone(), bytes.clone()).wait();
            }
            Mode::Slow(d) => {
                let (k, b) = (reply_key.clone(), bytes.clone());
                std::thread::spawn(move || {
                    std::thread::sleep(d);
                    let _ = q.reply(k, b).wait();
                });
            }
            Mode::Frozen => f.lock().unwrap().push(q),
        })
        .await
        .unwrap();
    Holder {
        _session: session,
        _q: q,
        _frozen: frozen,
    }
}

/// Waits until a raw `All` GET from `tool` gets `n` replies, so a test
/// starts once every holder's queryable has propagated.
async fn reachable(tool: &zenoh::Session, n: usize) {
    let imp = camera();
    let key = contract_key(imp.iface(), imp.fingerprint().hex())
        .unwrap()
        .into_keyexpr();
    let deadline = tokio::time::Instant::now() + SETTLE;
    loop {
        let rx = tool
            .get(key.clone())
            .target(QueryTarget::All)
            .consolidation(ConsolidationMode::None)
            .timeout(Duration::from_millis(300))
            .with(flume::unbounded())
            .await
            .unwrap();
        let mut got = 0;
        while rx.recv_async().await.is_ok() {
            got += 1;
        }
        if got >= n {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{got} of {n} holders reachable"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn fetch(tool: &zenoh::Session) -> (Retrieved, Duration) {
    let imp = camera();
    let t0 = Instant::now();
    let r = fetch_bundle(tool, imp.iface(), imp.fingerprint(), T)
        .await
        .unwrap();
    (r, t0.elapsed())
}

/// §1: one holder's valid reply is accepted; 200 equal holders on one
/// router answer a `BestMatching` GET once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_one_holder_many_holders() {
    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let one = holder(&ep, Mode::Good).await;
    reachable(&tool, 1).await;
    assert!(matches!(fetch(&tool).await.0, Retrieved::Bundle(..)));
    drop(one);

    let mut many = Vec::new();
    for _ in 0..200 {
        many.push(holder(&ep, Mode::Good).await);
    }
    reachable(&tool, 200).await;
    let imp = camera();
    let key = contract_key(imp.iface(), imp.fingerprint().hex())
        .unwrap()
        .into_keyexpr();
    let rx = tool
        .get(key)
        .target(QueryTarget::BestMatching)
        .consolidation(ConsolidationMode::None)
        .timeout(T)
        .with(flume::unbounded())
        .await
        .unwrap();
    let mut replies = 0;
    while rx.recv_async().await.is_ok() {
        replies += 1;
    }
    assert_eq!(replies, 1, "BestMatching reaches one of 200 equal holders");
}

/// §2: a slow, a frozen and a corrupt nearest holder; three good holders
/// behind R2. A valid bundle is accepted as its reply arrives.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_slow_unreachable_corrupt_nearest_holder() {
    for (what, mode) in [
        ("slow", Mode::Slow(Duration::from_secs(3))),
        ("frozen", Mode::Frozen),
        ("corrupt", Mode::Corrupt),
    ] {
        let (_r1, ep1) = router(None).await;
        let (_r2, ep2) = router(Some(&ep1)).await;
        let tool = client(&ep1).await;
        let _near = holder(&ep1, mode).await;
        let mut far = Vec::new();
        for _ in 0..3 {
            far.push(holder(&ep2, Mode::Good).await);
        }
        reachable(&tool, 3).await;
        let (got, took) = fetch(&tool).await;
        assert!(matches!(got, Retrieved::Bundle(..)), "{what}: {got:?}");
        assert!(
            took < T,
            "{what}: accepted after {took:?}, which waited for the GET to complete"
        );
    }
}

/// §3: every holder corrupt — unavailable after the retry, nothing
/// accepted; no holder near — the gateway's behind R2 is accepted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_no_valid_holder_a_constrained_holder() {
    let (_r1, ep1) = router(None).await;
    let (_r2, ep2) = router(Some(&ep1)).await;
    let tool = client(&ep1).await;
    {
        let _near = holder(&ep1, Mode::Corrupt).await;
        let mut far = Vec::new();
        for _ in 0..3 {
            far.push(holder(&ep2, Mode::Corrupt).await);
        }
        reachable(&tool, 4).await;
        match fetch(&tool).await.0 {
            Retrieved::Unavailable { refused } => assert!(!refused.is_empty()),
            Retrieved::Bundle(..) => panic!("a corrupt bundle was accepted"),
        }
    }
    // The nearest participant is constrained and holds none; a gateway
    // holds it behind R2.
    let _gateway = holder(&ep2, Mode::Good).await;
    reachable(&tool, 1).await;
    assert!(matches!(fetch(&tool).await.0, Retrieved::Bundle(..)));
}

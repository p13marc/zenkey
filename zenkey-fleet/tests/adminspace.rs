//! The admin space against a mesh that has none (issue #70).
//!
//! `admin.rs` has claimed since it was written that zero routers is "an empty
//! vec, **never** an error", and that a sweep answering nothing is "not
//! available" rather than "nothing declared". Both claims are load-bearing —
//! the whole admin panel's empty states rest on them — and neither had a test,
//! because a peer pair has no storage manager and there was nothing to assert
//! *against*.
//!
//! That is exactly what makes a peer pair the right fixture: zenoh's
//! `adminspace.enabled` defaults to false and `session::open` never turns it
//! on, so this is the deployment an operator most often runs an explorer
//! against, and the one where a panic or an `Err` would be worst.
//!
//! Self-contained: in-process peers, explicit endpoints, no scouting, no
//! router.
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.
//!
//! And `spec/scenarios/state.md` §12 (core §4.2, 0.48, #890): S4's check
//! reads against a router whose admin space is on and whose tables are
//! being written. Only the safe half runs here. zenoh 1.10.1 answers a
//! router's declaration records under its routing tables' read lock, and
//! a GET reaching them beside a writer freezes the router, and the suite
//! with it; `docs/zk2/upstream/adminspace-reply-repro/` measures that half
//! with zenoh alone, in a process of its own.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(2);

mod util;
use util::peer_pair;
use util::zk2::{client, config, iface};

/// A panic whose unwind hangs ends in seconds, with stacks (#804): what
/// §12 meets if a read ever freezes the router it shares a process with.
#[path = "../../zenkey/tests/common/panic_watchdog.rs"]
mod panic_watchdog;

/// Every admin call degrades to a *reading*, never to an error — which is what
/// lets an explorer render "peer-only mesh" instead of a failure dialog.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_admin_less_mesh_answers_empty_never_an_error() {
    let (_serving, asking) = peer_pair().await;

    let routers = zenkey_fleet::routers(&asking, TIMEOUT)
        .await
        .expect("zero routers is a reading, not an error");
    // If this build *does* serve a peer admin space, the rows must still be
    // well-formed — the zid fallback reads chunk 1 of the key.
    for r in &routers {
        assert!(!r.zid.is_empty(), "a router row always names its node");
    }

    let storages = zenkey_fleet::storages(&asking, TIMEOUT)
        .await
        .expect("zero storages is a reading, not an error");
    assert!(
        storages.is_empty(),
        "an in-process peer cannot host a storage manager: {storages:?}"
    );

    // The `Option` the admin panel's reachability sentence rests on.
    let declared = zenkey_fleet::declared_entities(&asking, TIMEOUT)
        .await
        .expect("an unreachable admin space is a reading, not an error");
    match declared {
        None => { /* nothing answered — the expected reading here */ }
        Some(e) => {
            // Tolerated rather than asserted against: if a future zenoh serves
            // a peer admin space, "answered with entities" is still a valid
            // reading. What must never happen is an `Err`.
            for entity in &e.entities {
                assert!(!entity.keyexpr.is_empty());
            }
        }
    }
}

/// state.md §12's rounds, its records ending in `**` beside the owner's,
/// and how long a put or a GET through the router may take after a read.
const ROUNDS: usize = 200;
const RECORDS: usize = 200;
const BOUND: Duration = Duration::from_secs(2);

/// A router on an ephemeral loopback port with its admin space on, read
/// permitted, and the endpoint it listens on.
async fn admin_router() -> (zenoh::Session, String) {
    let mut c = zenoh::Config::default();
    for (k, v) in [
        ("mode", "\"router\""),
        ("listen/endpoints", "[\"tcp/127.0.0.1:0\"]"),
        ("scouting/multicast/enabled", "false"),
        ("scouting/gossip/enabled", "false"),
        ("adminspace/enabled", "true"),
        ("adminspace/permissions/read", "true"),
    ] {
        c.insert_json5(k, v).expect("config");
    }
    let router = zenoh::open(c).await.expect("a router");
    let endpoint = util::bound(&router).await;
    (router, endpoint)
}

/// `spec/scenarios/state.md` §12 (core §4.2, 0.48): S4's check reads,
/// `@/*/router` and the storage manager's status subtree, 200 times
/// beside an owner whose state queryable ends in `**`, 400 more records
/// ending in `**`, and a client declaring flat out. Each read finds the
/// router and the storage; a put and the owner's S4 answer cross the
/// router within [`BOUND`] after each; and a queryable on `@/**` hears
/// each of the check's reads, two a round and nothing else, none reaching
/// a handler zenoh answers under its tables' read lock (it hears a read
/// only once the router routes it on, so a read that froze the router
/// would be missing from the count: F-254, core 0.50).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s12_the_checks_reads_beside_a_router_declaring() {
    panic_watchdog::install();
    let (router, ep) = admin_router().await;
    let router_zid = router.zid().to_string();

    // The owner: `ground/fleet-mgr` serving mission_plan.v1's state, its
    // queryable `…/mission_plan.v1/state/**`, holding `plans/vehicle-01`.
    let owners = client(&ep).await;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../zenkey/tests/contracts/mission_plan.v1.toml");
    let loaded = zenkey_model::contract::load_path(&path);
    let plan = loaded
        .contract
        .unwrap_or_else(|| panic!("mission_plan.v1 does not load:\n{}", loaded.report));
    let mut b = zenkey::ServiceBuilder::new(&owners, config("ground/fleet-mgr"));
    b.implement(zenkey::Implementation::new(plan.clone()))
        .expect("implement");
    for r in &plan.resources {
        let _ = b.expose(
            &iface("mission_plan.v1"),
            &zenkey::implementation::resource_name(r),
        );
    }
    b.serve_state(&iface("mission_plan.v1"))
        .expect("serve state");
    let mut owner = b.start().await.expect("the owner");
    let member: zenkey::model::template::Bindings =
        [("vehicle".to_owned(), vec!["vehicle-01".to_owned()])].into();
    let writer = owner
        .state_writer(&iface("mission_plan.v1"), "state/plans/{vehicle}", &member)
        .await
        .expect("a state writer");
    writer.put("go north").await.expect("a put");
    let plan_key = "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01";

    // The records a reply loop over the declarations would send.
    let loader = client(&ep).await;
    let mut records = Vec::new();
    for i in 0..RECORDS {
        let key = format!("load/{i}/**");
        records.push(
            Box::new(loader.declare_subscriber(&key).await.expect("a subscriber")) as Box<dyn Send>,
        );
        records.push(Box::new(
            loader
                .declare_queryable(&key)
                .callback(|_q| {})
                .await
                .expect("a queryable"),
        ));
    }

    // The storage manager's document, on the router's own key.
    let manager = client(&ep).await;
    let doc = format!("@/{router_zid}/router/status/plugins/storage_manager/storages/plans");
    let body = serde_json::to_vec(&serde_json::json!({
        "key_expr": "zk2/ground/fleet-mgr/**",
        "volume": "memory",
    }))
    .expect("JSON");
    let _storage = {
        let at = doc.clone();
        manager
            .declare_queryable(&doc)
            .callback(move |q| {
                use zenoh::Wait as _;
                let _ = q.reply(at.as_str(), body.clone()).wait();
            })
            .await
            .expect("the storage document")
    };

    // The ear on `@/**`: every admin GET's selector, answered with nothing.
    let ear = client(&ep).await;
    let heard: Arc<Mutex<Vec<String>>> = Arc::default();
    let _ear = {
        let heard = Arc::clone(&heard);
        ear.declare_queryable("@/**")
            .callback(move |q| heard.lock().unwrap().push(q.key_expr().to_string()))
            .await
            .expect("a queryable on @/**")
    };

    let reader = client(&ep).await;
    let (sender, receiver) = (client(&ep).await, client(&ep).await);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<u64>();
    let _hear = receiver
        .declare_subscriber("probe/round")
        .callback(move |s| {
            let n =
                u64::from_le_bytes(s.payload().to_bytes().as_ref().try_into().unwrap_or([0; 8]));
            let _ = tx.send(n);
        })
        .await
        .expect("a subscriber");
    // Every party heard through the router before the writer starts.
    let deadline = Instant::now() + util::SETTLE;
    loop {
        let _ = tokio::time::timeout(TIMEOUT, zenkey_fleet::routers(&reader, TIMEOUT)).await;
        let storages = tokio::time::timeout(TIMEOUT, zenkey_fleet::storages(&reader, TIMEOUT))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        let answered =
            match tokio::time::timeout(TIMEOUT, reader.get(plan_key).timeout(TIMEOUT)).await {
                Ok(Ok(replies)) => replies.recv_async().await.is_ok_and(|r| r.result().is_ok()),
                _ => false,
            };
        let _ = sender.put("probe/round", 0u64.to_le_bytes().to_vec()).await;
        let delivered = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .is_ok();
        if !heard.lock().unwrap().is_empty() && storages.len() == 1 && answered && delivered {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "never ready: ear {}, storages {storages:?}, owner {answered}, put {delivered}",
            heard.lock().unwrap().len()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    while rx.try_recv().is_ok() {}
    heard.lock().unwrap().clear();

    // The writer: a subscriber and a liveliness token declared and
    // undeclared, flat out, until the rounds are done.
    let churner = client(&ep).await;
    let stop = Arc::new(AtomicBool::new(false));
    let churns = Arc::new(AtomicU64::new(0));
    let churn = {
        let (stop, churns) = (Arc::clone(&stop), Arc::clone(&churns));
        tokio::spawn(async move {
            let mut n = 0u64;
            while !stop.load(Ordering::Relaxed) {
                n += 1;
                let key = format!("churn/{n}");
                if let Ok(s) = churner.declare_subscriber(&key).await {
                    let _ = s.undeclare().await;
                }
                if let Ok(t) = churner.liveliness().declare_token(&key).await {
                    let _ = t.undeclare().await;
                }
                churns.fetch_add(1, Ordering::Relaxed);
            }
        })
    };

    let (mut slowest_put, mut slowest_get) = (Duration::ZERO, Duration::ZERO);
    for round in 1..=ROUNDS as u64 {
        // 1. The check's reads.
        let routers =
            tokio::time::timeout(BOUND + TIMEOUT, zenkey_fleet::routers(&reader, TIMEOUT))
                .await
                .unwrap_or_else(|_| panic!("round {round}: @/*/router never ended"))
                .expect("a routers read");
        assert!(
            routers.iter().any(|r| r.zid == router_zid),
            "round {round}: the router answers @/*/router: {routers:?}"
        );
        let storages =
            tokio::time::timeout(BOUND + TIMEOUT, zenkey_fleet::storages(&reader, TIMEOUT))
                .await
                .unwrap_or_else(|_| panic!("round {round}: the storages read never ended"))
                .expect("a storages read");
        assert!(
            storages
                .iter()
                .any(|s| s.name == "plans"
                    && s.key_expr.as_deref() == Some("zk2/ground/fleet-mgr/**")),
            "round {round}: the storage is found: {storages:?}"
        );

        // 2. A put through the router.
        let sent = Instant::now();
        sender
            .put("probe/round", round.to_le_bytes().to_vec())
            .await
            .expect("a put");
        loop {
            let n = tokio::time::timeout(BOUND.saturating_sub(sent.elapsed()), rx.recv())
                .await
                .unwrap_or_else(|_| panic!("round {round}: the put never crossed the router"))
                .expect("the subscriber");
            if n == round {
                break;
            }
        }
        slowest_put = slowest_put.max(sent.elapsed());

        // 3. The owner's S4 answer through the router.
        let asked = Instant::now();
        let replies = reader
            .get(plan_key)
            .target(zenoh::query::QueryTarget::All)
            .consolidation(zenoh::query::ConsolidationMode::Latest)
            .timeout(BOUND)
            .await
            .expect("a GET");
        let reply = tokio::time::timeout(BOUND, replies.recv_async())
            .await
            .unwrap_or_else(|_| panic!("round {round}: the owner's answer never crossed"))
            .expect("a reply");
        let sample = reply.into_result().expect("the owner's value");
        assert_eq!(sample.payload().to_bytes().as_ref(), b"go north");
        slowest_get = slowest_get.max(asked.elapsed());
    }
    let churned = churns.load(Ordering::Relaxed);
    stop.store(true, Ordering::Relaxed);
    let _ = tokio::time::timeout(TIMEOUT, churn).await;
    assert!(
        churned > ROUNDS as u64,
        "the writer kept writing: {churned} rounds"
    );

    // 3. What the reads asked, as the ear heard them. The ear hears a
    //    read only once the router routes it on, so it never hears one
    //    that froze the router (F-254, 0.50): every read the check made is
    //    counted heard, two a round, and nothing else.
    let asked = heard.lock().unwrap().clone();
    assert_eq!(
        asked.len(),
        2 * ROUNDS,
        "the check's two reads a round, each heard: {asked:?}"
    );
    for read in [
        zenkey_fleet::bus::admin::ROUTERS,
        zenkey_fleet::bus::admin::STORAGES,
    ] {
        let n = asked.iter().filter(|s| *s == read).count();
        assert_eq!(n, ROUNDS, "{read} heard once a round");
    }
    assert!(
        asked.iter().all(|s| !zenkey_fleet::reaches_tables_lock(s)),
        "a read reached the tables' lock: {asked:?}"
    );
    eprintln!(
        "state.md §12: {ROUNDS} rounds beside {churned} churn rounds; slowest put {:?}, \
         slowest S4 answer {:?}",
        slowest_put, slowest_get
    );
    drop(records);
    drop(owner);
}

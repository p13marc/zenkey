//! A presence read is made where it can be complete (#830, #839; core §8.1,
//! 0.37): never beside a liveliness subscriber of its own session, never
//! through a session cut off from every other zenoh node, and a withdrawal
//! heard live is a reason to read again, never absence by itself.
//!
//! **The hazard** (#830, measured by `docs/zk2/upstream/liveliness-get-repro/`,
//! 61,920 trials per version on zenoh 1.10.1 and main at 766b455). A
//! liveliness GET from a session that holds a liveliness subscriber, history
//! or not, can complete without tokens that are live: the session's router
//! drops its answer for every token it already declared to the session for
//! the subscriber, and the session's own answer races the router's final
//! reply. A GET made right after a history subscriber on the same session
//! misses every time (720 of 720), which is what makes the tests here
//! deterministic: the first shows the hazard on purpose, so a zenoh that
//! fixes it is noticed, and the rest run the same sequence through the
//! fleet's wiring, which holds the subscribers on a watch session of their
//! own and finds every token.
//!
//! **The guard** is structural: every liveliness subscriber of the runtime
//! and the fleet is declared through `zenkey::presence::declare_subscriber`,
//! which marks its session, and every liveliness read on a marked session is
//! refused before anything is asked. Each reader that seeds beside a
//! subscription refuses the monitor's own session up front, typed
//! (`Error::NotAsked`), never a panic, never a silent fallback.
//!
//! No statistical test runs here: the deterministic sequence fails the old
//! wiring every time and the guard refuses it outright, so a bounded sample
//! of reads would test zenoh's two-session behaviour, not this crate. The
//! long run stays in the reproduction.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

mod util;
use util::zk2::{T, addr, client, client_ns, eventually, router, watch_of};

use zenkey_fleet::{
    AddressStatus, AlarmFeed, AlarmScope, AlarmSpec, LivePresence, Monitor, MonitorSpec, NotAsked,
    PresenceChange, PresenceFeed, PresenceScope, PresenceSource, PresenceTransition, SeedOutcome,
};

/// An instance token of `address`, through the grammar's own builder.
fn instance_token(address: &str, id: u64) -> String {
    zenkey_model::grammar::instance_key(
        &addr(address),
        &zenkey_model::grammar::InstanceId::from_u64(id),
    )
    .expect("a key")
    .as_str()
    .to_owned()
}

/// A presence monitor's spec over every service.
fn presence_spec() -> MonitorSpec {
    MonitorSpec {
        stats_tick: Duration::from_secs(3600),
        ..MonitorSpec::default()
    }
    .with_presence(&PresenceScope::all())
}

/// One raw liveliness GET on zenoh's own API, as the hazard needs it:
/// whether it completed (no error reply) and the keys it returned.
async fn raw_get(session: &zenoh::Session, selector: &str) -> (bool, Vec<String>) {
    let replies = session
        .liveliness()
        .get(selector)
        .timeout(T)
        .await
        .expect("a liveliness GET");
    let (mut complete, mut keys) = (true, Vec::new());
    while let Ok(reply) = replies.recv_async().await {
        match reply.result() {
            Ok(sample) => keys.push(sample.key_expr().as_str().to_owned()),
            Err(_) => complete = false,
        }
    }
    // A GET from the token's own session may hear it twice: the session's
    // own answer and the router's.
    keys.sort();
    keys.dedup();
    (complete, keys)
}

/// **The hazard, on purpose** (#830): zenoh's own API, no fleet in the way.
/// A liveliness GET issued right after a history liveliness subscriber is
/// declared on the same session completes without the live token. If this
/// fails, zenoh fixed #830's mechanism: revisit core §8.1's rule (0.37),
/// Appendix B and the watch sessions.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_liveliness_get_beside_a_history_subscriber_misses_a_live_token() {
    const ROUNDS: usize = 5;
    let (_router, ep) = router(None).await;
    let holder = client(&ep).await;
    let key = instance_token("h1/tc", 1);
    let _token = holder
        .liveliness()
        .declare_token(&key)
        .await
        .expect("a token");
    eventually("the router holds the token", || async {
        raw_get(&holder, "zk2/h1/tc/@zk/instance/*").await.1 == [key.clone()]
    })
    .await;
    let mut missed = 0;
    for _ in 0..ROUNDS {
        let reader = client(&ep).await;
        let _subscriber = reader
            .liveliness()
            .declare_subscriber("zk2/*/*/@zk/instance/*")
            .history(true)
            .callback(|_| {})
            .await
            .expect("a history subscriber");
        let (complete, keys) = raw_get(&reader, "zk2/*/*/@zk/**").await;
        // A fresh session in the same process finds it at once: the token
        // was live throughout.
        let fresh = client(&ep).await;
        assert_eq!(
            raw_get(&fresh, "zk2/*/*/@zk/**").await.1,
            std::slice::from_ref(&key)
        );
        if complete && !keys.contains(&key) {
            missed += 1;
        }
        let _ = fresh.close().await;
        let _ = reader.close().await;
    }
    assert!(
        missed > 0,
        "zenoh fixed #830's mechanism: revisit. A liveliness GET right after a history \
         liveliness subscriber on the same session found the live token in each of {ROUNDS} \
         rounds, where zenoh 1.10.1 missed 720 of 720: core §8.1's rule (0.37), Appendix B and \
         docs/zk2/upstream/liveliness-get-misses-beside-liveliness-subscriber.md may no longer \
         hold"
    );
}

/// **The fixed wiring, in the sequence that misses** (#830): a monitor's
/// history subscribers declared on a watch session, and the seed read at
/// once on another — the shape every reader of the fleet now has. Twenty
/// fresh pairs of sessions, and the seed finds the live token every time;
/// on one session, the hazard above misses it every time.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_seed_beside_a_monitor_of_its_own_finds_the_live_token_every_time() {
    const ROUNDS: usize = 20;
    let (_router, ep) = router(None).await;
    let holder = client(&ep).await;
    let key = instance_token("h1/tc", 1);
    let _token = holder
        .liveliness()
        .declare_token(&key)
        .await
        .expect("a token");
    eventually("the router holds the token", || async {
        raw_get(&holder, "zk2/h1/tc/@zk/instance/*").await.1 == [key.clone()]
    })
    .await;
    for round in 0..ROUNDS {
        let (watch, tool) = (client(&ep).await, client(&ep).await);
        let monitor = Monitor::start_watch(&watch_of(&watch, ""), presence_spec())
            .await
            .expect("a monitor");
        assert_eq!(monitor.settled(), Some(true), "round {round}");
        let (feed, _) =
            PresenceFeed::open(&tool, &monitor, LivePresence::new(PresenceScope::all()), T)
                .await
                .expect("the feed opens beside a monitor of its own");
        assert_eq!(*feed.last_seed(), SeedOutcome::Complete, "round {round}");
        assert_eq!(
            feed.presence().status(&addr("h1/tc")),
            AddressStatus::Up,
            "round {round}: the seed missed a live token (#830)"
        );
        drop(feed);
        monitor.shutdown().await.expect("shutdown");
        let _ = tool.close().await;
        let _ = watch.close().await;
    }
}

/// **The guard** (#830): a session that holds a liveliness subscriber the
/// fleet declared is a watch session, and makes no liveliness read — the
/// runtime's and the fleet's refuse it, typed, and a feed refuses the
/// monitor's own session and any other watch session — for as long as the
/// session lives, since its router keeps what it declared to it. The
/// deployment's session reads.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_watch_session_makes_no_presence_read() {
    let (_router, ep) = router(None).await;
    let (watch, other, tool) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let monitor = Monitor::start_watch(&watch_of(&watch, ""), presence_spec())
        .await
        .expect("a monitor");
    assert!(monitor.watches_liveliness());
    assert_eq!(monitor.zid(), watch.zid());
    assert!(zenkey::presence::watching(&watch));
    assert!(!zenkey::presence::watching(&tool));

    let sel = "zk2/*/*/@zk/**";
    let runtime = zenkey::presence::liveliness_read(&watch, sel, T)
        .await
        .expect_err("refused");
    assert!(
        matches!(&runtime, zenkey::Error::ReadBesideSubscriber { zid, .. } if *zid == watch.zid().to_string()),
        "{runtime}"
    );
    let fleet = zenkey_fleet::liveliness_read(&watch, sel, T)
        .await
        .expect_err("refused");
    assert!(
        matches!(fleet.not_asked(), Some(NotAsked::BesideSubscriber { .. })),
        "{fleet}"
    );
    assert!(fleet.to_string().contains("#830"), "{fleet}");
    let read = zenkey_fleet::read_tokens(&watch, &PresenceScope::all(), T).await;
    assert!(read.is_err(), "a read built on it is refused too");

    // The monitor's own session, as a feed's seed.
    let own = PresenceFeed::open(&watch, &monitor, LivePresence::new(PresenceScope::all()), T)
        .await
        .expect_err("refused");
    assert!(own.not_asked().is_some(), "{own}");
    // Another watch session.
    let second = Monitor::start_watch(&watch_of(&other, ""), presence_spec())
        .await
        .expect("a second monitor");
    let beside = PresenceFeed::open(&other, &monitor, LivePresence::new(PresenceScope::all()), T)
        .await
        .expect_err("refused");
    assert!(beside.not_asked().is_some(), "{beside}");

    // Undeclared, the subscribers leave the session a watch session.
    second.shutdown().await.expect("shutdown");
    assert!(zenkey_fleet::liveliness_read(&other, sel, T).await.is_err());

    // The deployment's session holds none, and reads.
    let (feed, _) = PresenceFeed::open(&tool, &monitor, LivePresence::new(PresenceScope::all()), T)
        .await
        .expect("the deployment's session reads");
    assert_eq!(*feed.last_seed(), SeedOutcome::Complete);
    drop(feed);
    monitor.shutdown().await.expect("shutdown");
}

/// `instance-gone`'s presence (the watchdog's, and `record --on`'s) needs a
/// watch session of its own: with none, or the session it reads through,
/// the run is refused before it watches anything (#830). With one it runs:
/// `tests/watchdog.rs`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn instance_gone_is_refused_without_a_watch_session_of_its_own() {
    use zenkey_fleet::Sipper as _;
    use zenkey_fleet::judge::condition::{Condition, WatchdogSpec, watchdog};

    let (_router, ep) = router(None).await;
    let tool = client(&ep).await;
    let spec = WatchdogSpec {
        rules: vec![Condition::parse("instance-gone h1/tc").expect("rule")],
        tick: Duration::from_millis(100),
        ticks: Some(1),
        timeout: T,
        clocks_synced: false,
    };
    let store = zenkey_fleet::BundleStore::new(T);
    let contracts = zenkey_fleet::ContractSet::new();
    for watch in [None, Some(watch_of(&tool, ""))] {
        let bus = zenkey_fleet::DoctorBus {
            session: tool.clone(),
            raw: tool.clone(),
            namespace: String::new(),
            watch,
        };
        let mut run = watchdog(&bus, &store, &contracts, &spec).pin();
        while run.sip().await.is_some() {}
        let refused = run.await.expect_err("refused");
        assert!(
            matches!(refused.not_asked(), Some(NotAsked::BesideSubscriber { .. })),
            "{refused}"
        );
    }
    assert!(
        !zenkey::presence::watching(&tool),
        "refused before anything was declared"
    );
}

/// An alarms reader that listens holds its owners' instance tokens on a
/// watch session of its own (#830): with none, or its read session, it is
/// refused; with one, the subscription is there and its reads are not. A
/// reader that does not listen needs none.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_alarm_feed_that_listens_needs_a_watch_session_of_its_own() {
    let (_router, ep) = router(None).await;
    let (tool, watch) = (client(&ep).await, client(&ep).await);
    let spec = AlarmSpec::new(T);
    let beside = watch_of(&tool, "");
    for refused in [None, Some(&beside)] {
        let e = AlarmFeed::open(&tool, refused, AlarmScope::all(), spec, true)
            .await
            .expect_err("refused");
        assert!(e.not_asked().is_some(), "{e}");
    }
    assert!(!zenkey::presence::watching(&tool));
    let (feed, _) = AlarmFeed::open(
        &tool,
        Some(&watch_of(&watch, "")),
        AlarmScope::all(),
        spec,
        true,
    )
    .await
    .expect("the reader opens");
    assert!(zenkey::presence::watching(&watch), "its subscription");
    assert!(!zenkey::presence::watching(&tool), "its reads");
    let log = feed.log();
    assert!(
        matches!(&log.presence, Some(Ok(seen)) if seen.complete),
        "the presence read was made, on the read session: {:?}",
        log.presence
    );
    let (_, _) = AlarmFeed::open(&tool, None, AlarmScope::all(), spec, false)
        .await
        .expect("a reader that does not listen");
}

/// A telemetry reading follows its owners' tokens on a watch session of its
/// own (#830): with none, or its read session, it is unobservable, saying
/// why, before it subscribes to anything; with one, the subscription is
/// there and its seed and reads are on the read session.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_telemetry_reading_needs_a_watch_session_of_its_own() {
    let (_router, ep) = router(None).await;
    let (owner, tool, watch) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let meter = {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zenkey/tests/contracts/meter.v1.toml");
        let l = zenkey_model::contract::load_path(&path);
        l.contract
            .unwrap_or_else(|| panic!("meter.v1 does not load:\n{}", l.report))
    };
    let mut b = zenkey::ServiceBuilder::new(&owner, util::zk2::config("lab/meter"));
    b.implement(zenkey::Implementation::new(meter.clone()))
        .expect("implement");
    for r in &meter.resources {
        let _ = b.expose(&meter.iface, &zenkey::implementation::resource_name(r));
    }
    let _svc = b.start().await.expect("the owner");
    // The owner's tokens travel on its own session, ordered against none
    // of the tool's: wait until the tool's router holds them (an instance
    // and an interface token) before a read counts on them.
    eventually("the tool sees the meter's tokens", || async {
        let (_, keys) = raw_get(&tool, "zk2/lab/meter/@zk/**").await;
        keys.iter().any(|k| k.contains("/@zk/instance/"))
            && keys.iter().any(|k| k.contains("/@zk/alive/"))
    })
    .await;
    let store = zenkey_fleet::BundleStore::new(T);
    let offline = zenkey_fleet::ContractSet::new();
    let mut spec = zenkey_fleet::TelemetrySpec::new(T, Duration::from_millis(200));
    spec.lease = Duration::from_millis(200);
    let read = |watch: Option<zenkey_fleet::WatchSession>| {
        let (tool, store, offline) = (tool.clone(), store.clone(), offline.clone());
        async move {
            zenkey_fleet::observe_telemetry(
                &tool,
                watch.as_ref(),
                "",
                zenkey_fleet::TelemetryScope::all(),
                &store,
                &offline,
                spec,
                |_| {},
            )
            .await
        }
    };
    for refused in [None, Some(watch_of(&tool, ""))] {
        let obs = read(refused).await;
        assert!(
            !obs.resources.is_empty(),
            "the meter's resources are in scope"
        );
        assert!(
            obs.unobservable
                .as_deref()
                .is_some_and(|why| why.contains("watch session")),
            "{:?}",
            obs.unobservable
        );
    }
    assert!(!zenkey::presence::watching(&tool));
    let obs = read(Some(watch_of(&watch, ""))).await;
    assert!(obs.unobservable.is_none(), "{:?}", obs.unobservable);
    assert!(
        matches!(&obs.seed, Some(Ok(seed)) if seed.complete),
        "the seed was read, on the read session: {:?}",
        obs.seed
    );
    assert!(zenkey::presence::watching(&watch), "its subscription");
    assert!(!zenkey::presence::watching(&tool), "its reads");
}

/// **A cut-off session's reads** (core §8.1, 0.37): a session that reaches
/// no other zenoh node is answered at once, complete and empty, though
/// nothing was asked of anyone. The fleet makes no such read, and says
/// why, and a feed over it is unobservable, never down.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cut_off_session_makes_no_presence_read() {
    let (tool, watch) = (alone().await, alone().await);
    // zenoh's own answer: complete and empty.
    assert_eq!(raw_get(&tool, "zk2/*/*/@zk/**").await, (true, Vec::new()));
    assert!(zenkey_fleet::cut_off(&tool).await.is_some());
    let e = zenkey_fleet::liveliness_read(&tool, "zk2/*/*/@zk/**", T)
        .await
        .expect_err("not made");
    assert_eq!(e.not_asked(), Some(&NotAsked::CutOff), "{e}");

    let monitor = Monitor::start_watch(&watch_of(&watch, ""), presence_spec())
        .await
        .expect("a monitor");
    let mut live = LivePresence::new(PresenceScope::all());
    live.track(&addr("h1/tc"), SystemTime::now());
    let (feed, _) = PresenceFeed::open(&tool, &monitor, live, T)
        .await
        .expect("the feed opens");
    assert!(
        matches!(feed.last_seed(), SeedOutcome::CutOff(why) if why.contains("no router and no peer")),
        "{:?}",
        feed.last_seed()
    );
    assert_eq!(feed.presence().basis(), zenkey_fleet::PresenceBasis::CutOff);
    assert_eq!(
        feed.presence().status(&addr("h1/tc")),
        AddressStatus::Unobservable,
        "never down"
    );
    drop(feed);
    monitor.shutdown().await.expect("shutdown");
}

/// A peer that reaches nobody: no listener, no scouting, no endpoint.
async fn alone() -> zenoh::Session {
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", "\"peer\"").unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    c.insert_json5("listen/endpoints", "[]").unwrap();
    zenoh::open(c).await.expect("a peer of nobody")
}

/// Folds the feed into `seen` until `want` holds of it, failing after
/// [`util::SETTLE`] with everything seen.
async fn until(
    feed: &mut PresenceFeed,
    seen: &mut Vec<PresenceTransition>,
    what: &str,
    want: impl Fn(&PresenceFeed) -> bool,
) {
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    while !want(feed) {
        match tokio::time::timeout_at(deadline, feed.next()).await {
            Ok(Some(moved)) => seen.extend(moved),
            Ok(None) => panic!("the monitor's broadcast closed"),
            Err(_) => panic!("never: {what}; saw {seen:#?}"),
        }
    }
}

// ─── a reader whose link is cut ─────────────────────────────────────────────

/// A TCP proxy to `upstream` that a test can cut, stall and heal: cutting
/// closes every forwarded connection and refuses new ones, so the sessions
/// behind it lose their router at once, as when it restarts; stalling stops
/// forwarding both ways and closes nothing.
struct Link {
    endpoint: String,
    cut: Arc<AtomicBool>,
    stalled: Arc<AtomicBool>,
    conns: Arc<std::sync::Mutex<Vec<tokio::task::AbortHandle>>>,
    _accept: tokio::task::JoinHandle<()>,
}

impl Link {
    async fn to(upstream: &str) -> Link {
        let target = upstream.trim_start_matches("tcp/").to_owned();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("tcp/{}", listener.local_addr().unwrap());
        let cut = Arc::new(AtomicBool::new(false));
        let stalled = Arc::new(AtomicBool::new(false));
        let conns: Arc<std::sync::Mutex<Vec<tokio::task::AbortHandle>>> = Arc::default();
        let (c, st, cs) = (Arc::clone(&cut), Arc::clone(&stalled), Arc::clone(&conns));
        let accept = tokio::spawn(async move {
            while let Ok((down, _)) = listener.accept().await {
                if c.load(Ordering::SeqCst) {
                    continue;
                }
                let Ok(up) = tokio::net::TcpStream::connect(&target).await else {
                    continue;
                };
                let (dr, dw) = down.into_split();
                let (ur, uw) = up.into_split();
                let (a, b) = (Arc::clone(&st), Arc::clone(&st));
                let h = tokio::spawn(async move {
                    tokio::join!(pump(dr, uw, a), pump(ur, dw, b));
                });
                cs.lock().unwrap().push(h.abort_handle());
            }
        });
        Link {
            endpoint,
            cut,
            stalled,
            conns,
            _accept: accept,
        }
    }

    fn cut(&self) {
        self.cut.store(true, Ordering::SeqCst);
        self.stalled.store(false, Ordering::SeqCst);
        for c in self.conns.lock().unwrap().drain(..) {
            c.abort();
        }
    }

    fn stall(&self) {
        self.stalled.store(true, Ordering::SeqCst);
    }

    fn heal(&self) {
        self.cut.store(false, Ordering::SeqCst);
        self.stalled.store(false, Ordering::SeqCst);
    }
}

/// Forwards `from` to `to` until either side closes, holding still while
/// `stalled` is set.
async fn pump(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    stalled: Arc<AtomicBool>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let hold = async || {
        while stalled.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        hold().await;
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        hold().await;
        if to.write_all(&buf[..n]).await.is_err() {
            break;
        }
    }
    let _ = to.shutdown().await;
}

/// A watch session behind `link` whose round trip cannot stand while the
/// link is stalled: its runtime ends a liveliness GET its router has not
/// finished at 300 ms (`routing/interests/timeout`), with no error reply,
/// and the runtime reads that as a round trip that did not come back
/// (#851).
async fn watch_behind(link: &Link) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    for (k, v) in [
        ("mode", "\"client\"".to_owned()),
        ("scouting/multicast/enabled", "false".to_owned()),
        ("scouting/gossip/enabled", "false".to_owned()),
        ("connect/endpoints", format!("[\"{}\"]", link.endpoint)),
        (
            "connect/retry",
            "{ period_init_ms: 100, period_max_ms: 500, period_increase_factor: 1.5 }".to_owned(),
        ),
        ("routing/interests/timeout", "300".to_owned()),
    ] {
        c.insert_json5(k, &v).unwrap();
    }
    zenoh::open(c).await.expect("a watch session")
}

/// A client of `endpoint` that retries fast after losing it.
async fn retrying_client(endpoint: &str) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .unwrap();
    c.insert_json5(
        "connect/retry",
        "{ period_init_ms: 100, period_max_ms: 500, period_increase_factor: 1.5 }",
    )
    .unwrap();
    zenoh::open(c).await.expect("a client")
}

/// **A link cut is not absence** (core §8.1, 0.37, 0.41): the reader's
/// link to its router is cut, as a router's restart cuts it, though the
/// owner never left. Its watch session's subscribers have no history, so
/// they were never told of the owner's token and hear no withdrawal of it
/// (#851): the watch session's transport closing is the reader's reason to
/// read again, and its barrier is lost with it; what it held is unconfirmed
/// from the move (0.42, F-229). The read is not made through a session cut
/// off, the address is unobservable, never down;
/// healed, the barrier is made again and the read finds the owner, up
/// again. A real departure is down, a read after its withdrawal.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reader_whose_link_is_cut_reads_nobody_down() {
    let (_router, ep) = router(None).await;
    let owner = client(&ep).await;
    let key = instance_token("h1/tc", 1);
    let token = owner
        .liveliness()
        .declare_token(&key)
        .await
        .expect("a token");
    let link = Link::to(&ep).await;
    let (watch, tool) = (
        retrying_client(&link.endpoint).await,
        retrying_client(&link.endpoint).await,
    );
    let h1 = addr("h1/tc");
    let monitor = Monitor::start_watch(&watch_of(&watch, ""), presence_spec())
        .await
        .expect("a monitor");
    let (mut feed, mut seen) =
        PresenceFeed::open(&tool, &monitor, LivePresence::new(PresenceScope::all()), T)
            .await
            .expect("the feed opens");
    assert_eq!(feed.presence().status(&h1), AddressStatus::Up);

    let downs = |s: &[PresenceTransition]| {
        s.iter()
            .filter(|t| {
                matches!(
                    t.change,
                    PresenceChange::Status {
                        to: AddressStatus::Down,
                        ..
                    }
                )
            })
            .count()
    };

    // Cut: the watch session's transport closes, which unconfirms what is
    // held at once (core §8.1, 0.42, F-229); the read it owes is not made,
    // and the address stays unobservable.
    link.cut();
    until(&mut feed, &mut seen, "a read not made", |f| {
        matches!(f.last_seed(), SeedOutcome::CutOff(_))
    })
    .await;
    assert!(
        seen.iter().any(|t| t.source == PresenceSource::Moved
            && t.change
                == PresenceChange::Status {
                    from: AddressStatus::Up,
                    to: AddressStatus::Unobservable
                }),
        "{seen:#?}"
    );
    assert_eq!(feed.presence().basis(), zenkey_fleet::PresenceBasis::CutOff);
    assert_eq!(
        monitor.settled(),
        Some(false),
        "the barrier went with the link"
    );
    assert_eq!(feed.presence().status(&h1), AddressStatus::Unobservable);
    assert_eq!(downs(&seen), 0, "nobody down: {seen:#?}");

    // Healed: the read finds the owner, up again.
    link.heal();
    until(&mut feed, &mut seen, "h1/tc up again", |f| {
        f.presence().status(&h1) == AddressStatus::Up
    })
    .await;
    assert_eq!(downs(&seen), 0, "nobody down: {seen:#?}");
    assert_eq!(*feed.last_seed(), SeedOutcome::Complete);
    assert_eq!(monitor.settled(), Some(true), "made again before the read");

    // A real departure is down, once the read after its withdrawal says so.
    token.undeclare().await.expect("undeclare");
    until(&mut feed, &mut seen, "h1/tc down", |f| {
        f.presence().status(&h1) == AddressStatus::Down
    })
    .await;
    let last = seen.last().expect("moves");
    assert_eq!(last.source, PresenceSource::Seed, "{seen:#?}");
    drop(feed);
    monitor.shutdown().await.expect("shutdown");
}

// ─── a round trip that does not stand (#851) ───────────────────────────────

/// `instance-gone`'s presence while its watch session's round trip does
/// not stand (#851, core §8.1, 0.41): the projection's seed is possibly
/// incomplete, so an address with no token reads unobservable, never
/// firing and never ok; once the round trip stands, the same rule fires.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn instance_gone_is_unobservable_while_its_round_trip_does_not_stand() {
    use zenkey_fleet::Sipper as _;
    use zenkey_fleet::judge::condition::{Condition, WatchdogSpec, watchdog};
    use zenkey_fleet::report::CondState;

    let (_router, ep) = router(None).await;
    let tool = client(&ep).await;
    let link = Link::to(&ep).await;
    let watch = watch_behind(&link).await;
    let spec = WatchdogSpec {
        rules: vec![Condition::parse("instance-gone lab/nobody").expect("rule")],
        tick: Duration::from_millis(200),
        ticks: Some(3),
        timeout: T,
        clocks_synced: false,
    };
    let store = zenkey_fleet::BundleStore::new(T);
    let contracts = zenkey_fleet::ContractSet::new();
    let bus = zenkey_fleet::DoctorBus {
        session: tool.clone(),
        raw: tool.clone(),
        namespace: String::new(),
        watch: Some(watch_of(&watch, "")),
    };
    let run = |bus: zenkey_fleet::DoctorBus| {
        let (store, contracts, spec) = (store.clone(), contracts.clone(), spec.clone());
        async move {
            let mut run = watchdog(&bus, &store, &contracts, &spec).pin();
            let mut transitions = Vec::new();
            while let Some(t) = run.sip().await {
                transitions.push(t);
            }
            (transitions, run.await.expect("a run"))
        }
    };

    link.stall();
    let (transitions, summary) = run(bus.clone()).await;
    assert!(
        transitions.iter().all(|t| t.to == CondState::Unobservable),
        "{transitions:#?}"
    );
    assert!(summary.firing.is_empty(), "{summary:?}");
    assert_eq!(
        summary.unobservable,
        ["instance-gone lab/nobody"],
        "{summary:?}"
    );

    link.heal();
    let (transitions, summary) = run(bus).await;
    assert_eq!(
        summary.firing,
        ["instance-gone lab/nobody"],
        "{summary:?} {transitions:#?}"
    );
}

/// An alarms reader that listens, while its watch session's round trip
/// does not stand (#851, core §8.1, 0.41): its presence read is possibly
/// incomplete, saying why, so the scope is not read whole and nothing
/// reads "no alarm fires"; its next refresh makes the round trip again,
/// and once it stands the scope is whole (here, with no owner in it).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_alarm_feed_whose_round_trip_does_not_stand_reads_the_scope_not_whole() {
    use zenkey_model::alarms::{Answer, Reason, ScopeRead, Severity};

    let (_router, ep) = router(None).await;
    let tool = client(&ep).await;
    let link = Link::to(&ep).await;
    let watch = watch_behind(&link).await;
    let spec = AlarmSpec::new(T);
    link.stall();
    let (mut feed, _) = AlarmFeed::open(
        &tool,
        Some(&watch_of(&watch, "")),
        AlarmScope::all(),
        spec,
        true,
    )
    .await
    .expect("the reader opens");
    let Some(Ok(seen)) = &feed.log().presence else {
        panic!("a presence read: {:?}", feed.log().presence)
    };
    assert!(!seen.complete && seen.unsettled, "{seen:?}");
    assert_eq!(
        seen.incomplete_reason(),
        Some(zenkey_fleet::BARRIER_NOT_ESTABLISHED)
    );
    assert_eq!(feed.book().scope_read(), ScopeRead::NotWhole);
    let any = feed
        .book()
        .any_firing(Severity::Info, std::time::Instant::now(), SystemTime::now());
    assert_eq!(
        (any.answer, any.reason),
        (Answer::Unobservable, Reason::ScopeIncomplete),
        "never no"
    );

    link.heal();
    feed.refresh().await;
    let Some(Ok(seen)) = &feed.log().presence else {
        panic!("a presence read: {:?}", feed.log().presence)
    };
    assert!(seen.complete && !seen.unsettled, "{seen:?}");
    assert_eq!(feed.book().scope_read(), ScopeRead::Whole);
    let any = feed
        .book()
        .any_firing(Severity::Info, std::time::Instant::now(), SystemTime::now());
    assert_eq!(
        (any.answer, any.reason),
        (Answer::NotAsked, Reason::NoOwner),
        "a whole scope with no owner in it"
    );
}

/// An alarms reader's watch session whose transport moves (core §8.1, 0.42,
/// F-229): its subscriber, without history, may not have heard an owner
/// leave or come while the link was down, so the move is a loss at once,
/// the scope not read whole and nothing "no alarm fires"; a refresh after
/// the link heals reads it whole again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_alarm_feed_counts_a_move_of_its_watch_session_as_a_loss() {
    use zenkey_model::alarms::ScopeRead;

    let (_router, ep) = router(None).await;
    let tool = client(&ep).await;
    let link = Link::to(&ep).await;
    let watch = retrying_client(&link.endpoint).await;
    let (mut feed, _) = AlarmFeed::open(
        &tool,
        Some(&watch_of(&watch, "")),
        AlarmScope::all(),
        AlarmSpec::new(T),
        true,
    )
    .await
    .expect("the reader opens");
    assert_eq!(feed.book().scope_read(), ScopeRead::Whole);
    assert_eq!(feed.book().lost(), 0);
    link.cut();
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    while feed.book().lost() == 0 {
        assert!(
            tokio::time::timeout_at(deadline, feed.next())
                .await
                .is_ok_and(|d| d.is_some()),
            "the move never folded"
        );
    }
    assert_eq!(feed.book().scope_read(), ScopeRead::NotWhole, "a loss");
    link.heal();
    eventually("the watch session back", || async {
        zenkey_fleet::cut_off(&watch).await.is_none()
    })
    .await;
    feed.refresh().await;
    assert_eq!(feed.book().scope_read(), ScopeRead::Whole, "healed");
}

// ─── in a namespace (#872, #875) ────────────────────────────────────────────

/// More instance tokens than a reader's queue holds
/// (`zenkey_fleet::bus::alarms::QUEUE`, telemetry's alike: 1,024).
const CROWD: usize = 1_100;

/// `n` instance tokens under system `crowd`, on `holder`, read whole by
/// `reader` before it returns.
async fn crowd(
    holder: &zenoh::Session,
    reader: &zenoh::Session,
    n: usize,
) -> Vec<zenoh::liveliness::LivelinessToken> {
    let mut tokens = Vec::with_capacity(n);
    for i in 0..n {
        tokens.push(
            holder
                .liveliness()
                .declare_token(instance_token(&format!("crowd/svc{i}"), i as u64 + 1))
                .await
                .expect("a token"),
        );
    }
    let sel = "zk2/crowd/*/@zk/instance/*";
    eventually("the crowd is read", || async {
        zenkey::presence::liveliness_read(reader, sel, Duration::from_secs(10))
            .await
            .is_ok_and(|r| r.complete && r.keys.len() == n)
    })
    .await;
    tokens
}

/// #875: in a namespace, a presence feed's monitor watches the wire keys
/// from a watch session in no namespace, its subscribers without history.
/// Beside more owners than a queue holds, its round trip stands at once,
/// every key reaches the feed base-relative (none refused), and an owner
/// up before the monitor is heard going: what #872 needed history for.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_presence_feed_in_a_namespace_watches_the_wire_keys() {
    let (_router, ep) = router(None).await;
    let holder = client_ns(&ep, Some("ns")).await;
    let tool = client_ns(&ep, Some("ns")).await;
    let raw = client(&ep).await;
    let mut held = crowd(&holder, &tool, CROWD).await;
    let watch = watch_of(&raw, "ns");
    assert!(!zenkey::presence::subscriber_needs_history(&raw));
    let monitor = Monitor::start_watch(&watch, presence_spec())
        .await
        .expect("a monitor");
    assert_eq!(monitor.settled(), Some(true), "one round trip");
    assert_eq!(
        monitor.barrier().map(zenkey::presence::Barrier::key),
        Some("ns/zk2/*/*/@zk/settle"),
        "the barrier asks the wire key"
    );
    let first = addr("crowd/svc0");
    let mut live = LivePresence::new(PresenceScope::all());
    live.track(&first, SystemTime::now());
    let (mut feed, _) = PresenceFeed::open(&tool, &monitor, live, Duration::from_secs(10))
        .await
        .expect("the feed opens");
    assert_eq!(*feed.last_seed(), SeedOutcome::Complete);
    assert_eq!(feed.presence().status(&first), AddressStatus::Up);
    held.remove(0).undeclare().await.expect("undeclare");
    let mut seen = Vec::new();
    until(&mut feed, &mut seen, "the owner read down", |f| {
        f.presence().status(&first) == AddressStatus::Down
    })
    .await;
    assert!(
        seen.iter().any(|t| t.address == first
            && t.source == PresenceSource::Withdrawn
            && matches!(
                t.change,
                PresenceChange::Status {
                    to: AddressStatus::Unobservable,
                    ..
                }
            )),
        "heard withdrawn first, then read down: {seen:#?}"
    );
    assert_eq!(
        feed.presence().ignored().total(),
        0,
        "every key base-relative"
    );
    assert_eq!(watch.outside(), 0, "none outside the namespace");
    drop(feed);
    monitor.shutdown().await.expect("shutdown");
}

/// #872, #875: in a namespace, an alarm feed's instance tokens are watched
/// on the wire keys from a watch session in no namespace, without history:
/// with more owners in scope than the queue holds, the feed opens having
/// lost nothing, its presence read complete, and hears an owner up before
/// it go.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_alarm_feed_in_a_namespace_watches_the_wire_keys() {
    use zenkey_model::alarms::Presence;
    let (_router, ep) = router(None).await;
    let holder = client_ns(&ep, Some("ns")).await;
    let tool = client_ns(&ep, Some("ns")).await;
    let watch = watch_of(&client(&ep).await, "ns");
    let mut held = crowd(&holder, &tool, CROWD).await;
    let scope = AlarmScope::parse("zk2/crowd/**").expect("a scope");
    let (mut feed, _) = AlarmFeed::open(
        &tool,
        Some(&watch),
        scope,
        AlarmSpec::new(Duration::from_secs(10)),
        true,
    )
    .await
    .expect("the reader opens");
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(300), feed.next()).await {}
    let Some(Ok(seen)) = &feed.log().presence else {
        panic!("a presence read: {:?}", feed.log().presence)
    };
    assert!(seen.complete && !seen.unsettled, "{seen:?}");
    assert_eq!(seen.services, CROWD, "{seen:?}");
    assert_eq!(feed.book().lost(), 0, "nothing lost");
    let first = addr("crowd/svc0");
    assert!(
        matches!(feed.book().presence_of(&first), Presence::Present(_)),
        "{:?}",
        feed.book().presence_of(&first)
    );
    held.remove(0).undeclare().await.expect("undeclare");
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    while feed.book().presence_of(&first) != Presence::Incomplete {
        assert!(
            tokio::time::timeout_at(deadline, feed.next())
                .await
                .is_ok_and(|d| d.is_some()),
            "never heard going: {:?}",
            feed.book().presence_of(&first)
        );
    }
    assert_eq!(feed.book().lost(), 0);
    assert_eq!(watch.outside(), 0, "none outside the namespace");
}

/// #872, #875: a telemetry reading in a namespace, its token subscriptions
/// on the wire keys from a watch session in no namespace, without history,
/// beside more instance tokens than its queue holds: the reading loses
/// nothing, and its seed stands.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_telemetry_reading_in_a_namespace_watches_the_wire_keys() {
    let (_router, ep) = router(None).await;
    let (owner, holder) = (
        client_ns(&ep, Some("ns")).await,
        client_ns(&ep, Some("ns")).await,
    );
    let tool = client_ns(&ep, Some("ns")).await;
    let watch = watch_of(&client(&ep).await, "ns");
    let _svc = meter(&owner).await;
    let _held = crowd(&holder, &tool, CROWD).await;
    eventually("the tool sees the meter", || async {
        raw_get(&tool, "zk2/lab/meter/@zk/instance/*").await.1.len() == 1
    })
    .await;
    let store = zenkey_fleet::BundleStore::new(Duration::from_secs(10));
    let offline = zenkey_fleet::ContractSet::new();
    let spec =
        zenkey_fleet::TelemetrySpec::new(Duration::from_secs(10), Duration::from_millis(300));
    let obs = zenkey_fleet::observe_telemetry(
        &tool,
        Some(&watch),
        "ns",
        zenkey_fleet::TelemetryScope::all(),
        &store,
        &offline,
        spec,
        |_| {},
    )
    .await;
    assert!(obs.unobservable.is_none(), "{:?}", obs.unobservable);
    assert!(
        matches!(&obs.seed, Some(Ok(seed)) if seed.complete && !seed.unsettled && seed.services == CROWD + 1),
        "{:?}",
        obs.seed
    );
    assert_eq!(obs.book.lost(), 0, "nothing lost");
    assert_eq!(watch.outside(), 0, "none outside the namespace");
}

/// The meter (`zenkey/tests/contracts/meter.v1.toml`) at `lab/meter`, on
/// `session`, every resource exposed.
async fn meter(session: &zenoh::Session) -> zenkey::Service {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../zenkey/tests/contracts/meter.v1.toml");
    let l = zenkey_model::contract::load_path(&path);
    let meter = l
        .contract
        .unwrap_or_else(|| panic!("meter.v1 does not load:\n{}", l.report));
    let mut b = zenkey::ServiceBuilder::new(session, util::zk2::config("lab/meter"));
    b.implement(zenkey::Implementation::new(meter.clone()))
        .expect("implement");
    for r in &meter.resources {
        let _ = b.expose(&meter.iface, &zenkey::implementation::resource_name(r));
    }
    b.start().await.expect("the owner")
}

/// #872 (WB's barrier, kept): a telemetry reading whose watch session's
/// round trip does not stand seeds anyway, its seed possibly incomplete
/// and saying so, and makes the round trip again before its next seed.
/// Stalled throughout, every seed says so; healed once the window opens,
/// the seed made again stands.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_telemetry_reading_whose_round_trip_does_not_stand_makes_it_again() {
    let (_router, ep) = router(None).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let _svc = meter(&owner).await;
    eventually("the tool sees the meter", || async {
        raw_get(&tool, "zk2/lab/meter/@zk/instance/*").await.1.len() == 1
    })
    .await;
    let link = Link::to(&ep).await;
    let watch = watch_behind(&link).await;
    let store = zenkey_fleet::BundleStore::new(T);
    let offline = zenkey_fleet::ContractSet::new();
    let watching = watch_of(&watch, "");
    let read = |window: Duration, heal: bool| {
        let (tool, watch, store, offline, link) = (&tool, &watching, &store, &offline, &link);
        async move {
            let spec = zenkey_fleet::TelemetrySpec::new(T, window);
            zenkey_fleet::observe_telemetry(
                tool,
                Some(watch),
                "",
                zenkey_fleet::TelemetryScope::all(),
                store,
                offline,
                spec,
                |_| {
                    if heal {
                        link.heal();
                    }
                },
            )
            .await
        }
    };
    link.stall();
    let settles = zenkey::presence::liveliness_gets(&watch).settles;
    let obs = read(Duration::from_millis(1500), false).await;
    assert!(
        obs.unobservable.is_none(),
        "it reads: {:?}",
        obs.unobservable
    );
    assert!(
        matches!(&obs.seed, Some(Ok(seed)) if !seed.complete && seed.unsettled && seed.services == 1),
        "possibly incomplete, what it read kept: {:?}",
        obs.seed
    );
    let report = zenkey_fleet::judge_telemetry(&obs);
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.contains(zenkey_fleet::BARRIER_NOT_ESTABLISHED)),
        "{:?}",
        report.skipped
    );
    let made = zenkey::presence::liveliness_gets(&watch).settles - settles;
    assert!(made >= 2, "made again within the window: {made}");

    let obs = read(Duration::from_millis(2500), true).await;
    assert!(
        matches!(&obs.seed, Some(Ok(seed)) if seed.complete && !seed.unsettled),
        "the seed made again stands: {:?}",
        obs.seed
    );
    assert_eq!(
        obs.book.presence_basis(),
        Some(zenkey_fleet::PresenceBasis::Complete)
    );
}

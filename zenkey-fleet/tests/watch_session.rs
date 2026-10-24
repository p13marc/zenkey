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
use util::zk2::{T, addr, client, eventually, router};

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
    .with_presence("", &PresenceScope::all())
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
        let monitor = Monitor::start(&watch, presence_spec())
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
    let monitor = Monitor::start(&watch, presence_spec())
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
    let second = Monitor::start(&other, presence_spec())
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
    for watch in [None, Some(tool.clone())] {
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
    for refused in [None, Some(&tool)] {
        let e = AlarmFeed::open(&tool, refused, AlarmScope::all(), spec, true)
            .await
            .expect_err("refused");
        assert!(e.not_asked().is_some(), "{e}");
    }
    assert!(!zenkey::presence::watching(&tool));
    let (feed, _) = AlarmFeed::open(&tool, Some(&watch), AlarmScope::all(), spec, true)
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
    let store = zenkey_fleet::BundleStore::new(T);
    let offline = zenkey_fleet::ContractSet::new();
    let mut spec = zenkey_fleet::TelemetrySpec::new(T, Duration::from_millis(200));
    spec.lease = Duration::from_millis(200);
    let read = |watch: Option<zenoh::Session>| {
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
    for refused in [None, Some(tool.clone())] {
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
    let obs = read(Some(watch.clone())).await;
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

    let monitor = Monitor::start(&watch, presence_spec())
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

/// A TCP proxy to `upstream` that a test can cut and heal: cutting closes
/// every forwarded connection and refuses new ones, so the sessions behind
/// it lose their router at once, as when it restarts.
struct Link {
    endpoint: String,
    cut: Arc<AtomicBool>,
    conns: Arc<std::sync::Mutex<Vec<tokio::task::AbortHandle>>>,
    _accept: tokio::task::JoinHandle<()>,
}

impl Link {
    async fn to(upstream: &str) -> Link {
        let target = upstream.trim_start_matches("tcp/").to_owned();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("tcp/{}", listener.local_addr().unwrap());
        let cut = Arc::new(AtomicBool::new(false));
        let conns: Arc<std::sync::Mutex<Vec<tokio::task::AbortHandle>>> = Arc::default();
        let (c, cs) = (Arc::clone(&cut), Arc::clone(&conns));
        let accept = tokio::spawn(async move {
            while let Ok((down, _)) = listener.accept().await {
                if c.load(Ordering::SeqCst) {
                    continue;
                }
                let Ok(up) = tokio::net::TcpStream::connect(&target).await else {
                    continue;
                };
                let (mut dr, mut dw) = down.into_split();
                let (mut ur, mut uw) = up.into_split();
                let h = tokio::spawn(async move {
                    let _ = tokio::join!(
                        tokio::io::copy(&mut dr, &mut uw),
                        tokio::io::copy(&mut ur, &mut dw)
                    );
                });
                cs.lock().unwrap().push(h.abort_handle());
            }
        });
        Link {
            endpoint,
            cut,
            conns,
            _accept: accept,
        }
    }

    fn cut(&self) {
        self.cut.store(true, Ordering::SeqCst);
        for c in self.conns.lock().unwrap().drain(..) {
            c.abort();
        }
    }

    fn heal(&self) {
        self.cut.store(false, Ordering::SeqCst);
    }
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

/// **A withdrawal heard live is not absence** (core §8.1, 0.37): the
/// reader's link to its router is cut, as a router's restart cuts it, and
/// the reader hears its owner's token withdrawn, though the owner never
/// left. The address is unobservable — first as heard withdrawn, then as a
/// read not made through a session cut off — and never down; healed, the
/// read finds the owner, and it is up again. A real departure is down, a
/// read after its withdrawal.
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
    let monitor = Monitor::start(&watch, presence_spec())
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

    // Cut: withdrawn, unobservable, and the read it owes is not made.
    link.cut();
    until(&mut feed, &mut seen, "a read not made", |f| {
        matches!(f.last_seed(), SeedOutcome::CutOff(_))
    })
    .await;
    assert!(
        seen.iter().any(|t| t.source == PresenceSource::Withdrawn
            && t.change
                == PresenceChange::Status {
                    from: AddressStatus::Up,
                    to: AddressStatus::Unobservable
                }),
        "{seen:#?}"
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

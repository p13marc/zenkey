//! `spec/scenarios/presence.md` §8 and §9, steps that observe the **reader
//! under test** (core §8.1, 0.38; F-188, F-189). The reference's reader is
//! this crate's [`PresenceFeed`] over a [`Monitor`]: its subscribers on a
//! watch session of their own, its reads on the tool's session. The
//! runtime cannot depend on the fleet, so `zenkey/tests/presence.rs` keeps
//! the halves of §8 and §9 that observe zenoh, and this file the halves
//! that observe the reader. One test per section, named after it; §7's
//! step 5 has the fleet's reader too.
//!
//! Reads are counted two ways: the feed's own count of the reads it made
//! ([`PresenceFeed::reads`]), and the runtime's count of the liveliness
//! GETs each session made (`zenkey::presence::liveliness_gets`), through
//! which every liveliness GET of the runtime and the fleet goes
//! (`scripts/check-liveliness-seam.sh`).
//!
//! Two sessions' messages are not ordered at their router (core §8.1), so
//! a read on one session of what another declared waits until the reading
//! session's router holds it (`eventually`), never assumes it.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

mod util;
use util::zk2::{T, addr, client, config, eventually, router};

use zenkey::presence::LivelinessGets;
use zenkey::{Implementation, Service, ServiceBuilder};
use zenkey_fleet::{
    AddressStatus, LivePresence, Monitor, MonitorSpec, NotAsked, PresenceChange, PresenceFeed,
    PresenceScope, PresenceSource, PresenceTransition, SeedOutcome,
};
use zenkey_model::contract::{Contract, load_path};

use AddressStatus::{Down, Unobservable, Up};

/// `ping.v1`, from the runtime's scenario contracts: one replicated
/// operation, so an owner holds an instance token and one interface token.
fn ping() -> Contract {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../zenkey/tests/contracts/ping.v1.toml");
    let l = load_path(&path);
    l.contract
        .unwrap_or_else(|| panic!("ping.v1 does not load:\n{}", l.report))
}

/// An owner of `ping.v1` at `address`, through the runtime's own builder.
async fn owner(session: &zenoh::Session, address: &str) -> Service {
    let c = ping();
    let mut b = ServiceBuilder::new(session, config(address));
    b.implement(Implementation::new(c.clone()))
        .expect("implement");
    for r in &c.resources {
        let _ = b.expose(&c.iface, &zenkey::implementation::resource_name(r));
    }
    b.start().await.expect("start")
}

/// A presence monitor's spec over every service.
fn presence_spec() -> MonitorSpec {
    MonitorSpec {
        stats_tick: Duration::from_secs(3600),
        ..MonitorSpec::default()
    }
    .with_presence("", &PresenceScope::all())
}

/// Whether `session`'s router shows it `key`, by one complete read.
async fn reads(session: &zenoh::Session, key: &str) -> bool {
    zenkey::presence::liveliness_read(session, key, T)
        .await
        .is_ok_and(|r| r.complete && r.keys.iter().any(|k| k == key))
}

/// Whether `session`'s router shows `key` gone, by one complete read.
async fn reads_gone(session: &zenoh::Session, key: &str) -> bool {
    zenkey::presence::liveliness_read(session, key, T)
        .await
        .is_ok_and(|r| r.complete && !r.keys.iter().any(|k| k == key))
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

/// Folds the feed into `seen` for `quiet`, whatever it hands out.
async fn fold_for(feed: &mut PresenceFeed, seen: &mut Vec<PresenceTransition>, quiet: Duration) {
    let deadline = tokio::time::Instant::now() + quiet;
    while let Ok(Some(moved)) = tokio::time::timeout_at(deadline, feed.next()).await {
        seen.extend(moved);
    }
}

/// `address`'s status moves in `seen`, with how each was learned.
fn statuses(
    seen: &[PresenceTransition],
    address: &str,
) -> Vec<(PresenceSource, AddressStatus, AddressStatus)> {
    seen.iter()
        .filter(|t| t.address.to_string() == address)
        .filter_map(|t| match t.change {
            PresenceChange::Status { from, to } => Some((t.source, from, to)),
            _ => None,
        })
        .collect()
}

/// Whether anything in `seen` reads an address down.
fn any_down(seen: &[PresenceTransition]) -> bool {
    seen.iter()
        .any(|t| matches!(t.change, PresenceChange::Status { to: Down, .. }))
}

// ─── a link the test can cut, stall and heal ────────────────────────────────

/// A TCP proxy to a router, as `zenkey/tests/common`'s `Link`.
/// - **Cutting** closes every forwarded connection and refuses new ones
///   until healed, so zenoh sees the link drop at once.
/// - **Stalling** stops forwarding both ways and closes nothing: the link
///   goes silent, and what is sent waits in it, in order, until healed.
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
                let (down_rd, down_wr) = down.into_split();
                let (up_rd, up_wr) = up.into_split();
                let (a, b) = (Arc::clone(&st), Arc::clone(&st));
                let h = tokio::spawn(async move {
                    tokio::join!(pump(down_rd, up_wr, a), pump(up_rd, down_wr, b));
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
/// `stalled` is set: nothing read, nothing written.
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

// ─── §7 ─────────────────────────────────────────────────────────────────────

/// §7 step 5 (core §8.1, 0.38, F-185), with the fleet's reader under test:
/// a re-read asked through a stalled link, and cut while it is in flight,
/// was made, and the feed reads it as any seed possibly incomplete. It
/// says the session was cut off while it ran, removes nothing, and leaves
/// the owner as the last complete read and the watch session's events
/// have it: up. The runtime's half is `zenkey/tests/presence.rs`'s
/// `s7_a_read_through_a_session_cut_off`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_a_read_cut_off_while_it_runs() {
    let (_r1, ep) = router(None).await;
    let owners = client(&ep).await;
    let link = Link::to(&ep).await;
    // The watch session reaches R1 directly: only the reads' link is cut.
    let (watch, tool) = (client(&ep).await, retrying_client(&link.endpoint).await);
    let a = owner(&owners, "p7/a").await;
    let key = a.instance_key().expect("a key").to_string();
    // Both its tokens, the interface token declared after the instance's
    // (core §8.2), before the seed counts on them.
    eventually("the tool reads both its tokens", || async {
        zenkey::presence::liveliness_read(&tool, "zk2/p7/a/@zk/**", T)
            .await
            .is_ok_and(|r| r.complete && r.keys.len() == 2 && r.keys.contains(&key))
    })
    .await;
    let monitor = Monitor::start(&watch, presence_spec())
        .await
        .expect("a monitor");
    let p7 = addr("p7/a");
    let mut live = LivePresence::new(PresenceScope::all());
    live.track(&p7, SystemTime::now());
    let (feed, _) = PresenceFeed::open(&tool, &monitor, live, T)
        .await
        .expect("the feed opens");
    assert_eq!(*feed.last_seed(), SeedOutcome::Complete);
    assert_eq!(feed.presence().status(&p7), Up);
    let made = feed.reads();

    link.stall();
    assert!(
        zenkey_fleet::cut_off(&tool).await.is_none(),
        "stalled, held"
    );
    let mut feed = feed;
    let reading = tokio::spawn(async move {
        let moved = feed.reseed().await;
        (feed, moved)
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!reading.is_finished(), "in flight, held by the stall");
    link.cut();
    let (feed, moved) = reading.await.unwrap();
    assert_eq!(feed.reads(), made + 1, "the read was made");
    assert!(
        matches!(feed.last_seed(), SeedOutcome::Incomplete(why) if why.contains("cut off while it ran")),
        "{:?}",
        feed.last_seed()
    );
    assert!(
        !moved.iter().any(|t| matches!(
            t.change,
            PresenceChange::Status { .. }
                | PresenceChange::InstanceGone { .. }
                | PresenceChange::InterfaceGone { .. }
        )),
        "nothing goes, and no status moves, on it: {moved:#?}"
    );
    assert_eq!(feed.presence().status(&p7), Up, "never read absent");
    assert_eq!(
        feed.presence().basis(),
        zenkey_fleet::PresenceBasis::Complete,
        "the complete read before it, and the events since, still stand"
    );

    link.heal();
    drop(feed);
    monitor.shutdown().await.expect("shutdown");
    a.close().await.expect("close");
}

// ─── §8 ─────────────────────────────────────────────────────────────────────

/// §8 (core §8.1, 0.37; its Expected on the reader since 0.38, F-188): a
/// withdrawal heard live is a reason to read again, never absence.
///
/// The tool's link is cut: the reader hears its owner withdrawn, though the
/// owner never left, and reports the address unobservable and withdrawn,
/// never down, its re-read not made. Healed, it reads up. A clean stop is
/// down only from the complete re-read after it. Three owners stopped at
/// once cost one read; a make-before-break re-mint, none.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s8_a_withdrawal_heard_live() {
    let (r1, ep) = router(None).await;
    let owners = client(&ep).await;
    let link = Link::to(&ep).await;
    let (watch, tool) = (
        retrying_client(&link.endpoint).await,
        retrying_client(&link.endpoint).await,
    );
    let monitor = Monitor::start(&watch, presence_spec())
        .await
        .expect("a monitor");
    assert_eq!(monitor.settled(), Some(true), "the round trip");
    let p8 = addr("p8/a");
    let mut live = LivePresence::new(PresenceScope::all());
    live.track(&p8, SystemTime::now());
    let (mut feed, mut seen) = PresenceFeed::open(&tool, &monitor, live, T)
        .await
        .expect("the feed opens");
    assert_eq!(*feed.last_seed(), SeedOutcome::Complete);

    // 1. The owner starts: heard, and read.
    let a = owner(&owners, "p8/a").await;
    let key = a.instance_key().expect("a key").to_string();
    until(&mut feed, &mut seen, "p8/a up", |f| {
        f.presence().status(&p8) == Up
    })
    .await;
    eventually("the tool reads it", || reads(&tool, &key)).await;

    // 2. Cut: heard withdrawn, the owner never left; unobservable, never
    // down, and the re-read not made.
    let made = feed.reads();
    let at_cut = seen.len();
    link.cut();
    until(&mut feed, &mut seen, "a re-read not made", |f| {
        matches!(f.last_seed(), SeedOutcome::CutOff(_))
    })
    .await;
    let rec = feed.presence().address(&p8).expect("tracked");
    assert_eq!(feed.presence().status(&p8), Unobservable, "{seen:#?}");
    assert!(rec.withdrawn, "withdrawn, awaiting a read: {rec:?}");
    assert!(
        statuses(&seen[at_cut..], "p8/a").contains(&(PresenceSource::Withdrawn, Up, Unobservable)),
        "{seen:#?}"
    );
    assert!(!any_down(&seen[at_cut..]), "never down: {seen:#?}");
    assert!(
        matches!(feed.last_seed(), SeedOutcome::CutOff(why) if why.contains("no router and no peer")),
        "{:?}",
        feed.last_seed()
    );
    assert!(reads(&r1, &key).await, "R1 reads it: the owner never left");
    let cut_reads = feed.reads() - made;

    // Healed: heard again, read, up.
    link.heal();
    until(&mut feed, &mut seen, "p8/a up again", |f| {
        f.presence().status(&p8) == Up
    })
    .await;
    assert!(
        !any_down(&seen[at_cut..]),
        "nobody down across the cut: {seen:#?}"
    );
    eventually("the tool reads it again", || reads(&tool, &key)).await;
    // The re-read that put it up is the first one made after the heal.
    until(
        &mut feed,
        &mut seen,
        "a complete read after the heal",
        |f| *f.last_seed() == SeedOutcome::Complete,
    )
    .await;
    assert!(
        feed.reads() > made + cut_reads,
        "a read made after the heal"
    );

    // 3. A clean stop: unobservable on the withdrawal, down only from the
    // complete re-read after it.
    let before = seen.len();
    let made = feed.reads();
    a.close().await.expect("close");
    until(&mut feed, &mut seen, "p8/a down", |f| {
        f.presence().status(&p8) == Down
    })
    .await;
    assert_eq!(
        statuses(&seen[before..], "p8/a"),
        [
            (PresenceSource::Withdrawn, Up, Unobservable),
            (PresenceSource::Seed, Unobservable, Down),
        ],
        "{seen:#?}"
    );
    assert!(feed.reads() > made, "down from a read");
    assert_eq!(*feed.last_seed(), SeedOutcome::Complete);
    assert!(reads_gone(&r1, &key).await, "it left");

    // 4. Three owners, stopped at once: one read, all three down from it.
    let three = ["p8/b", "p8/c", "p8/d"];
    let (b, c, d) = (
        owner(&owners, three[0]).await,
        owner(&owners, three[1]).await,
        owner(&owners, three[2]).await,
    );
    until(&mut feed, &mut seen, "the three up", |f| {
        three.iter().all(|a| f.presence().status(&addr(a)) == Up)
    })
    .await;
    fold_for(&mut feed, &mut seen, 3 * zenkey_fleet::WITHDRAWAL_SETTLE).await;
    assert!(
        !feed.presence().awaits_read(),
        "nothing owed before the stop"
    );
    let before = seen.len();
    let made = feed.reads();
    let (rb, rc, rd) = tokio::join!(b.close(), c.close(), d.close());
    rb.and(rc).and(rd).expect("close");
    until(&mut feed, &mut seen, "the three down", |f| {
        three.iter().all(|a| f.presence().status(&addr(a)) == Down)
    })
    .await;
    fold_for(&mut feed, &mut seen, 3 * zenkey_fleet::WITHDRAWAL_SETTLE).await;
    assert_eq!(feed.reads() - made, 1, "one read for the burst: {seen:#?}");
    let downs: Vec<_> = seen[before..]
        .iter()
        .filter(|t| matches!(t.change, PresenceChange::Status { to: Down, .. }))
        .collect();
    assert_eq!(downs.len(), 3, "{seen:#?}");
    assert!(
        downs
            .iter()
            .all(|t| t.source == PresenceSource::Seed && t.at == downs[0].at),
        "all three from the one read: {downs:#?}"
    );

    // 5. A make-before-break re-mint: no read, up throughout.
    let mut m = owner(&owners, "p8/m").await;
    let pm = addr("p8/m");
    until(&mut feed, &mut seen, "p8/m up", |f| {
        f.presence().status(&pm) == Up
    })
    .await;
    fold_for(&mut feed, &mut seen, 3 * zenkey_fleet::WITHDRAWAL_SETTLE).await;
    let old = m.instance().clone();
    let before = seen.len();
    let made = feed.reads();
    let new = m.new_epoch().await.expect("a re-mint");
    until(&mut feed, &mut seen, "the old instance gone", |f| {
        f.presence()
            .address(&pm)
            .is_some_and(|a| !a.instances.contains_key(&old) && a.instances.contains_key(&new))
    })
    .await;
    fold_for(&mut feed, &mut seen, 3 * zenkey_fleet::WITHDRAWAL_SETTLE).await;
    assert_eq!(feed.reads(), made, "no read for a re-mint: {seen:#?}");
    assert!(
        statuses(&seen[before..], "p8/m").is_empty(),
        "up throughout: {seen:#?}"
    );
    assert_eq!(feed.presence().status(&pm), Up);
    assert!(!feed.presence().awaits_read());
    assert!(
        seen[before..].iter().any(|t| t.change
            == PresenceChange::InstanceGone {
                instance: old.clone()
            }),
        "the old instance went: {seen:#?}"
    );

    m.close().await.expect("close");
    drop(feed);
    monitor.shutdown().await.expect("shutdown");
}

// ─── §9 ─────────────────────────────────────────────────────────────────────

/// How many fresh readers §9 step 5 starts: each one's watch link stalls
/// while it starts, so each run is a deterministic test, not a sample.
const FRESH_READERS: usize = 10;

/// How long a reader's watch link stays stalled while it starts.
const STALL: Duration = Duration::from_millis(150);

/// §9 (core §8.1, 0.37; steps 4 and 5 on the reader since 0.38, F-189): a
/// read beside a liveliness subscriber.
///
/// Step 4: the reader under test declares its subscribers on its watch
/// session, makes one liveliness GET there, the round trip, and no other;
/// its seed and re-reads go through the tool's session, and a read asked
/// on the watch session is refused.
///
/// Step 5: fresh readers, each with its owner's instance token withdrawn as
/// soon as its seed is answered, each hears it go and reads the owner down
/// after its re-read. Each reader's watch link is stalled while it starts,
/// so its subscribers' declarations reach the router late: the reference
/// waits for them (the round trip), and a reader that skips the round trip
/// seeds and is answered before its router has read them. The control
/// below is such a reader, and misses the withdrawal in every run.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s9_a_read_beside_a_liveliness_subscriber() {
    let (_r1, ep) = router(None).await;
    let owners = client(&ep).await;
    let watch_link = Link::to(&ep).await;

    // 4. Which session makes which GET.
    let o4 = owner(&owners, "p9/step4").await;
    let key = o4.instance_key().expect("a key").to_string();
    let tool = client(&ep).await;
    eventually("the tool reads the owner", || reads(&tool, &key)).await;
    let watch = retrying_client(&watch_link.endpoint).await;
    let tool_before = zenkey::presence::liveliness_gets(&tool);
    assert_eq!(
        zenkey::presence::liveliness_gets(&watch),
        LivelinessGets::default()
    );
    let monitor = Monitor::start(&watch, presence_spec())
        .await
        .expect("a monitor");
    assert_eq!(monitor.settled(), Some(true));
    assert_eq!(
        zenkey::presence::liveliness_gets(&watch),
        LivelinessGets {
            reads: 0,
            settles: 1
        },
        "after its subscribers, the round trip"
    );
    assert_eq!(
        zenkey::presence::liveliness_gets(&tool),
        tool_before,
        "and before the seed"
    );
    let p4 = addr("p9/step4");
    let mut live = LivePresence::new(PresenceScope::all());
    live.track(&p4, SystemTime::now());
    let (mut feed, mut seen) = PresenceFeed::open(&tool, &monitor, live, T)
        .await
        .expect("the feed opens");
    assert_eq!(feed.presence().status(&p4), Up);
    o4.close().await.expect("close");
    until(&mut feed, &mut seen, "p9/step4 down", |f| {
        f.presence().status(&p4) == Down
    })
    .await;
    let tool_after = zenkey::presence::liveliness_gets(&tool);
    assert!(feed.reads() >= 2, "a seed and a re-read: {}", feed.reads());
    assert_eq!(
        tool_after.reads - tool_before.reads,
        feed.reads(),
        "every read of the reader on the tool's session"
    );
    assert_eq!(tool_after.settles, 0);
    // A read asked on the watch session: not made, beside a subscriber.
    let refused = zenkey_fleet::liveliness_read(&watch, "zk2/*/*/@zk/**", T)
        .await
        .expect_err("refused");
    assert!(
        matches!(refused.not_asked(), Some(NotAsked::BesideSubscriber { .. })),
        "{refused}"
    );
    let own = PresenceFeed::open(&watch, &monitor, LivePresence::new(PresenceScope::all()), T)
        .await
        .expect_err("refused");
    assert!(own.not_asked().is_some(), "{own}");
    assert_eq!(
        zenkey::presence::liveliness_gets(&watch),
        LivelinessGets {
            reads: 0,
            settles: 1
        },
        "the round trip, and no other"
    );
    drop(feed);
    monitor.shutdown().await.expect("shutdown");

    // 5. Fresh readers, each withdrawn from as soon as its seed is answered.
    let started = std::time::Instant::now();
    for run in 0..FRESH_READERS {
        let address = format!("p9/run{run}");
        let a = addr(&address);
        let svc = owner(&owners, &address).await;
        let key = svc.instance_key().expect("a key").to_string();
        let tool = client(&ep).await;
        eventually("the tool reads the owner", || reads(&tool, &key)).await;
        let watch = retrying_client(&watch_link.endpoint).await;
        watch_link.stall();
        let starting = tokio::spawn({
            let watch = watch.clone();
            async move { Monitor::start(&watch, presence_spec()).await }
        });
        tokio::time::sleep(STALL).await;
        assert!(
            !starting.is_finished(),
            "run {run}: waiting on the round trip"
        );
        watch_link.heal();
        let monitor = starting.await.unwrap().expect("a monitor");
        assert_eq!(monitor.settled(), Some(true), "run {run}");
        let mut live = LivePresence::new(PresenceScope::all());
        live.track(&a, SystemTime::now());
        let (mut feed, mut seen) = PresenceFeed::open(&tool, &monitor, live, T)
            .await
            .expect("the feed opens");
        // As soon as the seed is answered.
        svc.close().await.expect("close");
        assert_eq!(feed.presence().status(&a), Up, "run {run}: the seed saw it");
        until(&mut feed, &mut seen, "the owner down", |f| {
            f.presence().status(&a) == Down
        })
        .await;
        assert_eq!(
            statuses(&seen, &address),
            [
                (PresenceSource::Seed, Unobservable, Up),
                (PresenceSource::Withdrawn, Up, Unobservable),
                (PresenceSource::Seed, Unobservable, Down),
            ],
            "run {run}: heard going, down after the re-read"
        );
        assert_eq!(
            zenkey::presence::liveliness_gets(&watch),
            LivelinessGets {
                reads: 0,
                settles: 1
            },
            "run {run}"
        );
        drop(feed);
        monitor.shutdown().await.expect("shutdown");
        let _ = watch.close().await;
        let _ = tool.close().await;
    }
    eprintln!(
        "§9 step 5: {FRESH_READERS} fresh readers in {:?}",
        started.elapsed()
    );

    // The control: a reader that skips the round trip, started the same
    // way. Its subscriber's declaration waits in the stalled link while its
    // seed is answered and the token goes, so it never hears the token go,
    // and its seed's "up" stands.
    for run in 0..3 {
        let address = format!("p9/control{run}");
        let svc = owner(&owners, &address).await;
        let key = svc.instance_key().expect("a key").to_string();
        let tool = client(&ep).await;
        eventually("the tool reads the owner", || reads(&tool, &key)).await;
        let watch = retrying_client(&watch_link.endpoint).await;
        let heard: Arc<std::sync::Mutex<BTreeSet<(String, bool)>>> = Arc::default();
        watch_link.stall();
        let h = Arc::clone(&heard);
        let _sub = zenkey::presence::declare_subscriber(&watch, "zk2/*/*/@zk/instance/*")
            .history(true)
            .callback(move |s: zenoh::sample::Sample| {
                h.lock().unwrap().insert((
                    s.key_expr().as_str().to_owned(),
                    s.kind() == zenoh::sample::SampleKind::Put,
                ));
            })
            .await
            .expect("a subscriber");
        let seed = zenkey::presence::liveliness_read(&tool, "zk2/*/*/@zk/instance/*", T)
            .await
            .expect("a seed");
        assert!(seed.complete && seed.keys.contains(&key), "run {run}");
        svc.close().await.expect("close");
        eventually("the token gone at the router", || reads_gone(&tool, &key)).await;
        watch_link.heal();
        assert!(
            zenkey::presence::settle(&watch, "zk2/*/*/@zk/instance/*", T)
                .await
                .expect("a barrier"),
            "run {run}: its router has read the subscriber now"
        );
        tokio::time::sleep(2 * zenkey_fleet::WITHDRAWAL_SETTLE).await;
        let heard = heard.lock().unwrap().clone();
        assert!(
            !heard.contains(&(key.clone(), false)),
            "run {run}: a reader without the round trip heard the withdrawal: {heard:?}"
        );
        let _ = watch.close().await;
        let _ = tool.close().await;
    }
}

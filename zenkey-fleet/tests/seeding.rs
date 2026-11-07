//! A late joiner's seed — core §4.2 (0.26) — proven against real zenoh
//! (#614, FL4; issue #42 before it, which proved v1's seed). The
//! subscription first, then the legs: S4's state GET (target `All`,
//! consolidation `Latest`) and the `@adv` history; replies and live
//! samples merged per key by the owner's stamp.
//!
//! Self-contained two-peer sessions, no router, except where a zk2 owner
//! is brought up through the runtime (a router and client sessions, as the
//! zk2 suites do it). `s10_a_late_joiners_seed` is
//! `spec/scenarios/state.md` §10.
//!
//! What the re-cut did to the cases v1 left here: none was deleted. The
//! "storage" queryables are state queryables now — a storage on an owner's
//! state is a deployment fault (S4), never a seed — and every assertion on
//! `storage_replies` reads the state leg.

use std::collections::BTreeMap;
use std::time::Duration;

use zenkey_fleet::report::{Asked, SeedLeg};
use zenkey_fleet::{SeedItem, SeedPolicy, seed_subscribe};
use zenoh::sample::SampleKind;
use zenoh::time::{NTP64, Timestamp, TimestampId};
use zenoh_ext::AdvancedPublisherBuilderExt;

mod util;
use util::timestamping_pair;

/// A sample's payload as every later step reads it (core §7.5): its
/// envelope's record, any other sample's bytes, as text; a refused
/// envelope's reason.
fn text(v: &zenkey_fleet::SampleView) -> String {
    match v.record() {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(r) => r.reason(),
    }
}

/// Drain a seeded subscriber until the boundary; returns (payloads, coverage).
async fn drain_seed(
    sub: &mut zenkey_fleet::SeededSubscriber,
) -> (Vec<String>, zenkey_fleet::SeedCoverage) {
    let mut values = Vec::new();
    loop {
        match tokio::time::timeout(util::SETTLE, sub.recv())
            .await
            .expect("seed boundary within 5s")
            .expect("stream alive")
        {
            SeedItem::Sample(v) => values.push(text(&v)),
            SeedItem::Dropped(n) => {
                panic!("these fixtures never outrun the bounded channel ({n} dropped)")
            }
            SeedItem::SeedComplete(c) => return (values, c),
        }
    }
}

/// The history seed reaches a live publisher's cache — and the boundary
/// arrives strictly AFTER the seed (the race this module exists to close:
/// a boundary that outruns the cache reply turns "loading" into "empty").
/// Live samples keep flowing after the boundary.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_seed_lands_before_the_boundary() {
    let (a, b) = timestamping_pair().await;

    let publisher = a
        .declare_publisher("seedtest/state/health")
        .cache(zenoh_ext::CacheConfig::default().max_samples(1))
        .await
        .expect("advanced publisher");
    publisher.put("v1").await.expect("cached put");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut sub = seed_subscribe(
        &b,
        "seedtest/state/**",
        SeedPolicy {
            timeout: Duration::from_millis(800),
            ..SeedPolicy::default()
        },
    )
    .await
    .expect("seed subscribe");

    let (seen, coverage) = drain_seed(&mut sub).await;
    assert_eq!(seen, ["v1"], "the cached value seeds — before the boundary");
    assert_eq!(
        coverage.history.as_option().map(|l| l.replies),
        Some(1),
        "the cache answered once"
    );
    assert_eq!(
        coverage.state.as_option().map(|l| l.replies),
        Some(0),
        "no state queryable on this bus — ran and found nothing, an observation"
    );

    // Live after the boundary.
    publisher.put("v2").await.expect("live put");
    let item = tokio::time::timeout(util::SETTLE, sub.recv())
        .await
        .expect("live within 5s")
        .expect("stream alive");
    match item {
        SeedItem::Sample(v) => assert_eq!(v.payload.to_bytes().as_ref(), b"v2"),
        other => panic!("expected the live sample, got {other:?}"),
    }
}

/// The merge suppresses a stale reply: a queryable whose (unstamped) reply
/// to the state GET arrives after the stamped cache value must not regress
/// the key — and the suppression is counted, never silent (O6). No zk2
/// owner answers unstamped (S1, S2): this is a foreign key's replier, which
/// the state leg reaches as well (0.26). The reply is delayed so the order
/// is deterministic: stamped value first, stale echo second.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_reply_cannot_regress_a_key() {
    let (a, b) = timestamping_pair().await;

    // The publisher's cache holds the CURRENT value, stamped now.
    let publisher = a
        .declare_publisher("staletest/state/doc")
        .cache(zenoh_ext::CacheConfig::default().max_samples(1))
        .await
        .expect("advanced publisher");
    publisher.put("current").await.expect("put");

    // A replier that answers late and UNstamped — by the time it replies,
    // the stamped cache value has already seeded the key; the merge must
    // refuse the regression (stamped state beats an unstamped echo).
    let _replier = a
        .declare_queryable("staletest/state/doc")
        .callback(move |query| {
            let q = query.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                q.reply("staletest/state/doc", "stale-unstamped").await.ok();
            });
        })
        .await
        .expect("queryable");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut sub = seed_subscribe(
        &b,
        "staletest/state/**",
        SeedPolicy {
            timeout: Duration::from_millis(800),
            ..SeedPolicy::default()
        },
    )
    .await
    .expect("seed subscribe");

    let (values, coverage) = drain_seed(&mut sub).await;
    assert_eq!(
        values,
        ["current"],
        "the stamped cache value seeds once; the late unstamped echo never surfaces"
    );
    assert_eq!(
        coverage.state.as_option().map(|l| l.replies),
        Some(1),
        "the replier DID answer"
    );
    assert!(
        coverage.superseded >= 1,
        "…and its suppression is counted, not silent (O6)"
    );
}

/// The gap rule: a sample published immediately after `seed_subscribe`
/// returns — racing the seed GETs — arrives exactly once. This is the
/// transition GET-then-subscribe silently drops.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_transition_in_the_seed_window_lands_exactly_once() {
    let (a, b) = timestamping_pair().await;
    let publisher = a
        .declare_publisher("gaptest/state/flag")
        .await
        .expect("publisher");
    let matching = publisher
        .matching_listener()
        .await
        .expect("matching listener");
    // A state queryable that holds the GET open (replying nothing) and says
    // when it arrived — the put below waits for both signals, so the seed
    // window is *provably* open when the transition is published, even on a
    // loaded machine.
    let (got_query_tx, mut got_query) = tokio::sync::mpsc::unbounded_channel::<()>();
    let _slow_state = a
        .declare_queryable("gaptest/state/**")
        .callback(move |query| {
            let _ = got_query_tx.send(());
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(2000)).await;
                drop(query);
            });
        })
        .await
        .expect("queryable");
    // A ready-probe declared AFTER the state queryable: same-session
    // declarations propagate in order, so once b can query this, b can query
    // the state queryable — the seed GET below provably reaches it, loaded
    // machine or not.
    let _ready = a
        .declare_queryable("gaptest/ready")
        .callback(|query| {
            let q = query.clone();
            tokio::spawn(async move {
                q.reply("gaptest/ready", "ok").await.ok();
            });
        })
        .await
        .expect("ready queryable");
    let probe_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let replies = b
            .get("gaptest/ready")
            .timeout(Duration::from_millis(300))
            .await
            .expect("probe get");
        if replies.recv_async().await.is_ok() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < probe_deadline,
            "routing never converged"
        );
    }

    let mut sub = seed_subscribe(
        &b,
        "gaptest/state/**",
        SeedPolicy {
            history: false, // plain publisher; the subscriber-first order is the point
            timeout: Duration::from_millis(3000),
            ..SeedPolicy::default()
        },
    )
    .await
    .expect("seed subscribe");

    // Wait until (1) the subscriber's interest reached the publishing peer
    // (a network-propagation concern, not part of the seed contract) and
    // (2) the state GET is in flight — then publish inside the window.
    let ev = tokio::time::timeout(util::SETTLE, matching.recv_async())
        .await
        .expect("matching event within 5s")
        .expect("listener alive");
    assert!(ev.matching(), "the seed subscriber is a real subscriber");
    tokio::time::timeout(util::SETTLE, got_query.recv())
        .await
        .expect("the state GET reaches the queryable within 5s")
        .expect("channel alive");
    publisher.put("flank").await.expect("put");

    let mut before_boundary = 0;
    let mut after_boundary = 0;
    let mut done = false;
    let mut deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        match tokio::time::timeout_at(deadline, sub.recv()).await {
            Ok(Some(SeedItem::Sample(v))) => {
                assert_eq!(v.payload.to_bytes().as_ref(), b"flank");
                if done {
                    after_boundary += 1;
                } else {
                    before_boundary += 1;
                }
            }
            Ok(Some(SeedItem::Dropped(n))) => {
                panic!("this fixture never outruns the bounded channel ({n} dropped)")
            }
            Ok(Some(SeedItem::SeedComplete(c))) => {
                assert_eq!(c.history, Asked::NotAsked, "history was opted out");
                done = true;
                // Short drain: anything late (a duplicate, a post-boundary
                // copy) has this long to show itself.
                deadline = tokio::time::Instant::now() + Duration::from_millis(700);
            }
            Ok(None) | Err(_) => break,
        }
    }
    assert!(done, "the seed boundary must arrive");
    assert_eq!(
        (before_boundary, after_boundary),
        (1, 0),
        "the in-window transition lands exactly once, inside the seed phase"
    );
}

/// Issue #92's acceptance: a seeded watch on `…/state/**` against a
/// publisher whose cached value predates the watch shows that value without
/// waiting for a refresh — through the monitor's own bounded broadcast, with
/// the boundary as a typed event carrying this watch's id and coverage.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_seeded_watch_shows_pre_existing_state() {
    let (a, b) = timestamping_pair().await;
    let publisher = a
        .declare_publisher("wseed/state/health")
        .cache(zenoh_ext::CacheConfig::default().max_samples(1))
        .await
        .expect("advanced publisher");
    publisher.put("cached-before-watch").await.expect("put");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let monitor = zenkey_fleet::Monitor::start(&b, zenkey_fleet::MonitorSpec::default())
        .await
        .expect("monitor");
    let mut events = monitor.events();
    let id = monitor
        .watch_seeded(
            "wseed/state/**",
            SeedPolicy {
                timeout: Duration::from_millis(800),
                ..SeedPolicy::default()
            },
        )
        .await
        .expect("seeded watch");

    // Drain until the boundary; the cached value must arrive before it.
    let mut seen = Vec::new();
    let coverage = loop {
        let item = tokio::time::timeout(util::SETTLE, events.recv())
            .await
            .expect("boundary within 5s")
            .expect("stream alive");
        match item {
            zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::Sample(s)) => {
                seen.push(String::from_utf8_lossy(&s.payload.to_bytes()).to_string());
            }
            zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::WatchSeeded {
                id: seeded,
                coverage,
            }) => {
                assert_eq!(seeded, id, "the boundary names the watch it closes");
                break coverage;
            }
            _ => {}
        }
    };
    assert_eq!(
        seen,
        ["cached-before-watch"],
        "pre-existing state arrives without waiting for a refresh"
    );
    assert_eq!(coverage.history.as_option().map(|l| l.replies), Some(1));
    assert_eq!(coverage.state.as_option().map(|l| l.replies), Some(0));

    // …and the seeded key is already in the tree at the boundary tick.
    assert_eq!(monitor.tree().keys, 1);

    // Live samples keep flowing after the boundary: newer than what the
    // key holds, so the merge, kept for the watch's life, keeps it.
    publisher.put("live-after").await.expect("live put");
    loop {
        let item = tokio::time::timeout(util::SETTLE, events.recv())
            .await
            .expect("live within 5s")
            .expect("stream alive");
        if let zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::Sample(s)) = item {
            assert_eq!(s.payload.to_bytes().as_ref(), b"live-after");
            break;
        }
    }
}

/// #342: dropping a monitor aborts its seed tasks too.
///
/// `Drop` aborted only `self.tasks`; a seeded watch's task lives in `watches`
/// and was merely dropped, which detaches. It holds a cloned `Session` and
/// goes on ingesting until its own seed timeout — one per dropped monitor, in
/// a GUI that rebuilds its monitor on every re-scope.
///
/// The boundary event is the tell: an aborted seed task never reaches the
/// `WatchSeeded` send, a detached one does. The `EventStream` outlives the
/// monitor (it holds the core), so it is still listening either way.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_a_monitor_aborts_its_seed_tasks() {
    let (a, b) = timestamping_pair().await;

    // **The seed has to be genuinely pending**, and nothing else here makes
    // it so. `seed_get` loops until the reply channel closes, and with no
    // queryable at all zenoh closes it as soon as the query resolves — so
    // the boundary fired in microseconds and `SEED_TIMEOUT` never entered
    // into it. The test then passed only when the machine was loaded enough
    // to hold the query past the premise window, which is not a test.
    //
    // A queryable that takes the query and never answers holds it open until
    // the *querier's* timeout, so the seed reliably runs the full
    // `SEED_TIMEOUT` and the abort below is what decides the outcome.
    let _blocker = a
        .declare_queryable("wdrop/state/**")
        .callback(move |query| {
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(30)).await;
                drop(query);
            });
        })
        .await
        .expect("blocking queryable");
    // A ready-probe declared AFTER the blocker, the same device
    // `a_transition_in_the_seed_window_lands_exactly_once` uses: same-session
    // declarations propagate in order, so once `b` can query this, `b` can
    // reach the blocker. Without it the seed GET can go out before the
    // blocker is visible, resolve against nobody, and void the premise —
    // which is the second way this test was environmental.
    let _ready = a
        .declare_queryable("wdrop/ready")
        .callback(|query| {
            let q = query.clone();
            tokio::spawn(async move {
                q.reply("wdrop/ready", "ok").await.ok();
            });
        })
        .await
        .expect("ready queryable");
    let probe_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let replies = b
            .get("wdrop/ready")
            .timeout(Duration::from_millis(300))
            .await
            .expect("probe get");
        if replies.recv_async().await.is_ok() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < probe_deadline,
            "routing never converged"
        );
    }

    let monitor = zenkey_fleet::Monitor::start(&b, zenkey_fleet::MonitorSpec::default())
        .await
        .expect("monitor");
    let mut events = monitor.events();

    // What the *querier* waits before giving up on the blocker above — so it
    // is now what the seed's duration actually is, rather than an upper bound
    // nothing approached.
    const SEED_TIMEOUT: Duration = Duration::from_secs(2);
    monitor
        .watch_seeded(
            "wdrop/state/**",
            SeedPolicy {
                timeout: SEED_TIMEOUT,
                ..SeedPolicy::default()
            },
        )
        .await
        .expect("seeded watch");

    // The premise, asserted rather than assumed: the boundary has not fired
    // yet, so the abort below is what decides the outcome. If this ever trips,
    // the machine was slow enough to void the test — which is a legible
    // failure, unlike the silent one it replaces.
    //
    // **Everything queued, not one item.** The stream also carries
    // `StatsTick` (every 250 ms) and `WatchChanged`, so reading a single
    // item could consume one of those, leave a `WatchSeeded` queued behind
    // it, and pass a premise that is false — after which the loop below
    // pulls that boundary and reports it as having "outlived the monitor"
    // when it fired *before* the drop. That is this check's own failure
    // wearing the other check's message, and it is what made this test flake
    // under a loaded `--workspace` run while passing 12 times in isolation.
    while let Ok(Some(item)) = tokio::time::timeout(Duration::from_millis(50), events.recv()).await
    {
        assert!(
            !matches!(
                item,
                zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::WatchSeeded { .. })
            ),
            "the seed announced before the monitor was dropped — the premise is void, \
             not the code (raise SEED_TIMEOUT)"
        );
    }

    // The blocker holds the seed GET, so the task would otherwise run its
    // timeout out and then announce the boundary.
    drop(monitor);

    // Listen past the timeout by a clear margin: a *detached* task announces
    // at ~SEED_TIMEOUT, so a window shorter than that would pass either way
    // and prove nothing.
    let listen = tokio::time::Instant::now() + SEED_TIMEOUT * 2;
    while let Ok(Some(item)) = tokio::time::timeout_at(listen, events.recv()).await {
        assert!(
            !matches!(
                item,
                zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::WatchSeeded { .. })
            ),
            "the seed task outlived the monitor that owned it"
        );
    }
}

// ─── core §4.2's late joiner (0.26) ─────────────────────────────────────────

/// An owner the test plays (`spec/scenarios/state.md` §10): its state
/// queryable over `zk2/ground/<service>/mission_plan.v1/state/**` hands
/// every GET to the test, which answers when it chooses, with the stamps it
/// mints — one timestamp id, the owner session's zid, as §4.3 has it.
struct HeldOwner {
    session: zenoh::Session,
    service: &'static str,
    queries: tokio::sync::mpsc::UnboundedReceiver<zenoh::query::Query>,
    _queryable: zenoh::query::Queryable<()>,
    _ready: zenoh::query::Queryable<()>,
    base: u64,
    id: TimestampId,
}

/// `mission_plan.v1.Plan`'s predefined Encoding (core §7.2): a put of
/// a plan under zenoh's default, `zenoh/bytes`, would carry a zk2c
/// envelope (§7.5) and read as a refused one, never merged.
const PLAN: zenoh::bytes::Encoding = zenoh::bytes::Encoding::APPLICATION_PROTOBUF;

impl HeldOwner {
    /// Declares the queryable, then a ready-probe after it, and waits until
    /// `joiner` reaches the probe: same-session declarations propagate in
    /// order, so the joiner's GET then reaches the queryable.
    async fn new(owner: &zenoh::Session, joiner: &zenoh::Session, service: &'static str) -> Self {
        let (tx, queries) = tokio::sync::mpsc::unbounded_channel();
        let queryable = owner
            .declare_queryable(format!("zk2/ground/{service}/mission_plan.v1/state/**"))
            .callback(move |q| {
                let _ = tx.send(q);
            })
            .await
            .expect("the owner's state queryable");
        let probe = format!("zk2/ground/{service}/ready");
        let ready = owner
            .declare_queryable(probe.clone())
            .callback({
                let probe = probe.clone();
                move |q| {
                    let probe = probe.clone();
                    tokio::spawn(async move {
                        q.reply(probe, "ok").await.ok();
                    });
                }
            })
            .await
            .expect("ready queryable");
        let deadline = tokio::time::Instant::now() + util::SETTLE;
        loop {
            let replies = joiner
                .get(probe.clone())
                .timeout(Duration::from_millis(300))
                .await
                .expect("probe get");
            if replies.recv_async().await.is_ok() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "routing never converged"
            );
        }
        HeldOwner {
            session: owner.clone(),
            service,
            queries,
            _queryable: queryable,
            _ready: ready,
            base: owner.new_timestamp().get_time().as_u64(),
            id: TimestampId::from(owner.zid()),
        }
    }

    /// Tn: the owner's n-th stamp, in order.
    fn stamp(&self, n: u64) -> Timestamp {
        Timestamp::new(NTP64(self.base + n), self.id)
    }

    fn key(&self, plan: &str) -> String {
        format!(
            "zk2/ground/{}/mission_plan.v1/state/plans/{plan}",
            self.service
        )
    }

    fn selector(&self) -> String {
        self.key("*")
    }

    async fn put(&self, plan: &str, value: &str, n: u64) {
        self.session
            .put(self.key(plan), value.to_owned())
            .encoding(PLAN)
            .timestamp(self.stamp(n))
            .await
            .expect("the owner's put");
    }

    /// Waits until a subscription on the plans reaches the owner.
    async fn matched(&self) {
        let publisher = self
            .session
            .declare_publisher(self.key("probe"))
            .await
            .expect("publisher");
        let deadline = tokio::time::Instant::now() + util::SETTLE;
        while !publisher
            .matching_status()
            .await
            .expect("matching status")
            .matching()
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the subscription never reached the owner"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn next_query(&mut self) -> zenoh::query::Query {
        tokio::time::timeout(util::SETTLE, self.queries.recv())
            .await
            .expect("the GET reaches the owner")
            .expect("queryable alive")
    }
}

/// What a late joiner holds of one key: its last admitted sample's kind,
/// payload and stamp, and how many samples were admitted for it.
#[derive(Debug, PartialEq)]
struct Held {
    kind: SampleKind,
    value: String,
    stamp: Option<Timestamp>,
    admitted: usize,
}

/// `spec/scenarios/state.md` §10: a late joiner's seed (core §4.2, 0.26).
///
/// The late joiner subscribes, then GETs, through the fleet's seeded
/// subscription. The owner holds the GET: it puts `plans/d` (T3), makes its
/// answer (`plans/a` v1 at T1, a `reply_del` for `plans/b` at T2, `plans/d`
/// at T3), puts `plans/a` v2 (T4) and `plans/c` (T5), then answers. The
/// late joiner holds a = v2 (the stale reply lost to the newer put and was
/// counted), b deleted at T2 (the `reply_del` merged as a deletion), c (the
/// put the answer lacks, kept by the subscription), and d once (one
/// mutation, two paths); its state leg ran to its final reply. The control
/// — GET, then subscribe — loses `plans/c`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s10_a_late_joiners_seed() {
    let (owner, joiner) = timestamping_pair().await;
    let mut o = HeldOwner::new(&owner, &joiner, "fleet-mgr").await;

    // 1. Subscribe, then GET (the seeded subscription's own order).
    let mut sub = seed_subscribe(
        &joiner,
        &o.selector(),
        SeedPolicy {
            history: false,
            state: true,
            timeout: util::SETTLE,
        },
    )
    .await
    .expect("seed subscribe");

    // 2. The owner takes the GET, holds it, and puts plans/d.
    let q = o.next_query().await;
    o.matched().await;
    o.put("d", "d-v1", 3).await;
    // 3. Its answer is made now: plans/a v1 (T1), plans/b deleted (T2),
    //    plans/d (T3). 4. Before sending it, two more puts.
    o.put("a", "a-v2", 4).await;
    o.put("c", "c-v1", 5).await;
    // 5. The answer, then the GET's end.
    q.reply(o.key("a"), "a-v1")
        .encoding(PLAN)
        .timestamp(o.stamp(1))
        .await
        .expect("reply");
    q.reply_del(o.key("b"))
        .timestamp(o.stamp(2))
        .await
        .expect("reply_del");
    q.reply(o.key("d"), "d-v1")
        .encoding(PLAN)
        .timestamp(o.stamp(3))
        .await
        .expect("reply");
    drop(q);

    let mut held: BTreeMap<String, Held> = BTreeMap::new();
    let coverage = loop {
        let item = tokio::time::timeout(util::SETTLE, sub.recv())
            .await
            .expect("the seed's boundary")
            .expect("stream alive");
        match item {
            SeedItem::Sample(v) => {
                let plan = v.key.rsplit('/').next().expect("a chunk").to_owned();
                let admitted = held.get(&plan).map_or(0, |h| h.admitted) + 1;
                held.insert(
                    plan,
                    Held {
                        kind: v.kind,
                        value: String::from_utf8_lossy(&v.payload.to_bytes()).into_owned(),
                        stamp: v.timestamp,
                        admitted,
                    },
                );
            }
            SeedItem::Dropped(n) => panic!("this fixture never outruns the channel ({n})"),
            SeedItem::SeedComplete(c) => break c,
        }
    };
    let get = |plan: &str| {
        held.get(plan)
            .unwrap_or_else(|| panic!("plans/{plan} is not held: {held:#?}"))
    };
    // Expected 1: the stale reply lost to the newer live put.
    assert_eq!(
        get("a"),
        &Held {
            kind: SampleKind::Put,
            value: "a-v2".into(),
            stamp: Some(o.stamp(4)),
            admitted: 1,
        },
        "the reply (T1) never shows after v2 (T4)"
    );
    // Expected 2: the put the answer lacks, through the subscription.
    assert_eq!(get("c").value, "c-v1");
    assert_eq!(get("c").stamp, Some(o.stamp(5)));
    // Expected 3: the reply_del, a deletion with its stamp.
    assert_eq!(
        get("b").kind,
        SampleKind::Delete,
        "a deletion, not an absence"
    );
    assert_eq!(get("b").stamp, Some(o.stamp(2)));
    // Expected 4: one mutation, two paths, kept once.
    assert_eq!(get("d").admitted, 1, "the live put and the reply are one");
    assert_eq!(get("d").stamp, Some(o.stamp(3)));
    assert_eq!(held.len(), 4);
    // Expected 5: the state leg ran to its final reply.
    assert_eq!(
        coverage.state,
        Asked::Asked(SeedLeg {
            replies: 3,
            complete: true,
            snapshots: Vec::new(),
        })
    );
    assert_eq!(coverage.history, Asked::NotAsked);
    assert_eq!(
        coverage.superseded, 2,
        "the stale plans/a reply and plans/d's second copy, counted (O6)"
    );

    // 6. The control: GET, then subscribe, on the bare session.
    let mut ctl = HeldOwner::new(&owner, &joiner, "fleet-mgr-ctl").await;
    let replies = joiner
        .get(ctl.selector())
        .target(zenoh::query::QueryTarget::All)
        .consolidation(zenoh::query::ConsolidationMode::Latest)
        .timeout(util::SETTLE)
        .await
        .expect("the control's GET");
    let q = ctl.next_query().await;
    // The answer is made (plans/a, plans/b deleted); then plans/c, then the
    // answer is sent.
    ctl.put("c", "c-v1", 5).await;
    q.reply(ctl.key("a"), "a-v1")
        .encoding(PLAN)
        .timestamp(ctl.stamp(1))
        .await
        .expect("reply");
    q.reply_del(ctl.key("b"))
        .timestamp(ctl.stamp(2))
        .await
        .expect("reply_del");
    drop(q);
    let mut answered = Vec::new();
    while let Ok(reply) = replies.recv_async().await {
        if let Ok(sample) = reply.result() {
            answered.push(sample.key_expr().as_str().to_owned());
        }
    }
    answered.sort();
    assert_eq!(answered, [ctl.key("a"), ctl.key("b")]);
    let (tx, mut live) = tokio::sync::mpsc::unbounded_channel::<String>();
    let _late = joiner
        .declare_subscriber(ctl.selector())
        .callback(move |s| {
            let _ = tx.send(s.key_expr().as_str().to_owned());
        })
        .await
        .expect("the control's subscription");
    ctl.matched().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(500), live.recv())
            .await
            .is_err(),
        "nothing arrives: plans/c was put before the subscription existed"
    );
}

/// What a seed cannot know (0.26): a state GET that reaches its timeout
/// seeds possibly incompletely, and its leg says so — zenoh ends such a GET
/// with an error reply (core Appendix B). The owner here takes the GET and
/// never answers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_state_get_that_reaches_its_timeout_seeds_incompletely() {
    let (owner, joiner) = timestamping_pair().await;
    let mut o = HeldOwner::new(&owner, &joiner, "fleet-mgr").await;
    let mut sub = seed_subscribe(
        &joiner,
        &o.selector(),
        SeedPolicy {
            history: false,
            state: true,
            timeout: Duration::from_millis(500),
        },
    )
    .await
    .expect("seed subscribe");
    let held = o.next_query().await;
    let (seen, coverage) = drain_seed(&mut sub).await;
    drop(held);
    assert!(seen.is_empty(), "nothing answered, nothing seeded");
    assert_eq!(
        coverage.state,
        Asked::Asked(SeedLeg {
            replies: 0,
            complete: false,
            snapshots: Vec::new(),
        }),
        "ran to its timeout: possibly incomplete, never \"no state\" (O5)"
    );
}

/// The state leg against a zk2 owner the runtime brings up (`tc.netif.v1`
/// at `host-a/tc`, its state served): the seeded watch zengui's tree uses
/// receives the owner's current value, with its last confirmation, the
/// owner's own stamp, and, for the key it deleted within the window, a
/// `reply_del` — a deletion with the deletion's in-band stamp (S2, S3).
/// `tc.netif.v1` uses `freshness.v2` since #978's F5, so neither its put
/// nor its delete carried a Zenoh timestamp (core §4.3's table).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_seeded_watch_seeds_from_a_zk2_owner() {
    use util::zk2::{T, client, config, eventually, example, iface, router};
    use zenkey::{Implementation, ServiceBuilder};

    let (_router, ep) = router(None).await;
    let owners = client(&ep).await;
    let raw = client(&ep).await;
    let netif = iface("tc.netif.v1");
    let contract = example("tcgui/tc.netif.v1");
    let mut b = ServiceBuilder::new(&owners, config("host-a/tc"));
    b.implement(Implementation::new(contract.clone()))
        .expect("implement");
    for r in &contract.resources {
        let _ = b.expose(&netif, &zenkey::implementation::resource_name(r));
    }
    b.serve_state(&netif).expect("serve state");
    let mut service = b.start().await.expect("the owner");
    let member = |name: &str| -> zenkey_model::template::Bindings {
        [
            ("ns".to_owned(), vec!["default".to_owned()]),
            ("iface".to_owned(), vec![name.to_owned()]),
        ]
        .into()
    };
    let resource = "state/interfaces/{ns}/{iface}";
    let eth0 = service
        .state_writer(&netif, resource, &member("eth0"))
        .await
        .expect("state writer");
    let eth1 = service
        .state_writer(&netif, resource, &member("eth1"))
        .await
        .expect("state writer");
    let put0 = eth0
        .put(br#"{"name":"eth0","index":2,"namespace":"default","is_up":true}"#.to_vec())
        .await
        .expect("put");
    eth1.put(br#"{"name":"eth1","index":3,"namespace":"default","is_up":false}"#.to_vec())
        .await
        .expect("put");
    let deleted = eth1.delete().await.expect("delete");

    let selector = "zk2/host-a/tc/tc.netif.v1/state/**";
    eventually("the owner's state answers", || async {
        zenkey_fleet::fleet_get(&raw, selector, &zenkey_fleet::GetOpts::new(T))
            .await
            .is_ok_and(|a| !a.is_empty())
    })
    .await;

    let monitor = zenkey_fleet::Monitor::start(&raw, zenkey_fleet::MonitorSpec::default())
        .await
        .expect("monitor");
    let mut events = monitor.events();
    let id = monitor
        .watch_seeded(
            selector,
            SeedPolicy {
                timeout: T,
                ..SeedPolicy::default()
            },
        )
        .await
        .expect("seeded watch");
    let mut seen = BTreeMap::new();
    let coverage = loop {
        let item = tokio::time::timeout(util::SETTLE, events.recv())
            .await
            .expect("the boundary")
            .expect("stream alive");
        match item {
            zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::Sample(s)) => {
                let inband = s.envelope().and_then(Result::ok);
                seen.insert(s.key.clone(), (s.kind, s.timestamp, inband));
            }
            zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::WatchSeeded {
                id: seeded,
                coverage,
            }) => {
                assert_eq!(seeded, id);
                break coverage;
            }
            _ => {}
        }
    };
    let owner = TimestampId::from(owners.zid());
    let key = |name: &str| format!("zk2/host-a/tc/tc.netif.v1/state/interfaces/default/{name}");
    // `interfaces` declares `freshness.ttl_s` and uses freshness.v2: its
    // mutations carry no Zenoh timestamp, and a reply of a live member, or
    // a deleted one's reply_del, carries its last confirmation, the
    // owner's (S2, core §4.3's table).
    assert_eq!(
        (put0.time, deleted.time),
        (None, None),
        "a freshness.v2 state's put and delete carry no Zenoh timestamp"
    );
    let (kind, confirmed, inband) = seen.get(&key("eth0")).expect("eth0's reply");
    assert_eq!(
        (kind, inband),
        (&SampleKind::Put, &Some(put0.stamp)),
        "the current value, with its in-band stamp (S2)"
    );
    // Read by snapshot (core 0.99, #977's E3): its entry carries no Zenoh
    // timestamp; its confirmation rides in band.
    assert_eq!(
        confirmed, &None,
        "a snapshot's entry carries no Zenoh timestamp"
    );
    let (kind, tombstone, inband) = seen.get(&key("eth1")).expect("eth1's reply_del");
    assert_eq!(
        (kind, inband),
        (&SampleKind::Delete, &Some(deleted.stamp)),
        "the reply_del, a deletion with the deletion's in-band stamp (S3)"
    );
    assert_eq!(tombstone, &None, "a snapshot's tombstone carries none");
    let _ = owner;
    // `state/**` reads each collection whole: one page each, `interfaces`'
    // and `namespaces`', on their snapshot keys, complete.
    let Asked::Asked(leg) = &coverage.state else {
        panic!("the state leg ran: {coverage:?}")
    };
    assert_eq!((leg.replies, leg.complete), (2, true), "{leg:?}");
    assert_eq!(leg.snapshots.len(), 2, "{leg:?}");
    let interfaces = leg
        .snapshots
        .iter()
        .find(|s| s.key == "zk2/host-a/tc/tc.netif.v1/state/interfaces")
        .expect("interfaces' snapshot");
    assert!(interfaces.complete && interfaces.pages == [0]);
    monitor.shutdown().await.expect("shutdown");
    service.close().await.expect("close");
}

/// A foreign key's writer (`spec/scenarios/state.md` §11): a session with
/// no HLC, so its puts arrive unstamped unless it sets a stamp, answering
/// GETs on `foreign/**` as a storage would — every GET handed to the test,
/// which answers when it chooses.
struct Foreign {
    session: zenoh::Session,
    queries: tokio::sync::mpsc::UnboundedReceiver<zenoh::query::Query>,
    _queryable: zenoh::query::Queryable<()>,
    base: u64,
}

impl Foreign {
    /// Declares the queryable, then waits until `joiner` reaches it.
    async fn new(writer: &zenoh::Session, joiner: &zenoh::Session) -> Self {
        let (tx, queries) = tokio::sync::mpsc::unbounded_channel();
        let queryable = writer
            .declare_queryable("foreign/**")
            .callback(move |q| {
                if q.key_expr().as_str() == "foreign/ready" {
                    tokio::spawn(async move {
                        q.reply("foreign/ready", "ok").await.ok();
                    });
                } else {
                    let _ = tx.send(q);
                }
            })
            .await
            .expect("the writer's queryable");
        let deadline = tokio::time::Instant::now() + util::SETTLE;
        loop {
            let replies = joiner
                .get("foreign/ready")
                .timeout(Duration::from_millis(300))
                .await
                .expect("probe get");
            if replies.recv_async().await.is_ok() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "routing never converged"
            );
        }
        Foreign {
            session: writer.clone(),
            queries,
            _queryable: queryable,
            base: writer.new_timestamp().get_time().as_u64(),
        }
    }

    /// A stamp `n` ticks past the writer's clock, with the id `id` (hex).
    fn stamp(&self, n: u64, id: &str) -> Timestamp {
        Timestamp::new(NTP64(self.base + n), id.parse().expect("an id"))
    }

    async fn put(&self, key: &str, value: &str, stamp: Option<Timestamp>) {
        let mut put = self.session.put(format!("foreign/{key}"), value.to_owned());
        if let Some(ts) = stamp {
            put = put.timestamp(ts);
        }
        put.await.expect("the writer's put");
    }

    /// Waits until a subscription on `foreign/**` reaches the writer.
    async fn matched(&self) {
        let publisher = self
            .session
            .declare_publisher("foreign/probe")
            .await
            .expect("publisher");
        let deadline = tokio::time::Instant::now() + util::SETTLE;
        while !publisher
            .matching_status()
            .await
            .expect("matching status")
            .matching()
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the subscription never reached the writer"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn next_query(&mut self) -> zenoh::query::Query {
        tokio::time::timeout(util::SETTLE, self.queries.recv())
            .await
            .expect("the GET reaches the writer")
            .expect("queryable alive")
    }
}

/// One admitted sample: its key's last chunk, kind, payload and stamp.
type Admitted = (String, SampleKind, String, Option<Timestamp>);

/// The next sample a seeded subscription admits.
async fn next_admitted(sub: &mut zenkey_fleet::SeededSubscriber) -> Admitted {
    loop {
        let item = tokio::time::timeout(util::SETTLE, sub.recv())
            .await
            .expect("a sample")
            .expect("stream alive");
        match item {
            SeedItem::Sample(v) => {
                return (
                    v.key.strip_prefix("foreign/").unwrap_or(&v.key).to_owned(),
                    v.kind,
                    String::from_utf8_lossy(&v.payload.to_bytes()).into_owned(),
                    v.timestamp,
                );
            }
            SeedItem::Dropped(n) => panic!("this fixture never outruns the channel ({n})"),
            SeedItem::SeedComplete(_) => {}
        }
    }
}

/// `spec/scenarios/state.md` §11, steps 1–4: an unstamped sample ranks
/// below every stamp (core §4.2, 0.27, #752). No zk2 owner: a foreign
/// key's writer, which stamps only when it sets a stamp, answers the seed's
/// GET. Expected: an unstamped live put on a key holding nothing is kept;
/// two unstamped samples of a key have no order but their arrival, so the
/// later is kept, even an older reply arriving last; a stamped reply
/// replaces an unstamped live put; once a key holds a stamp, an unstamped
/// put or delete is not kept, and counted, and a newer stamp still is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s11_an_unstamped_sample_ranks_below_every_stamp() {
    let (writer, joiner) = util::peer_pair().await;
    let mut w = Foreign::new(&writer, &joiner).await;
    let mut sub = seed_subscribe(
        &joiner,
        "foreign/**",
        SeedPolicy {
            history: false,
            state: true,
            timeout: util::SETTLE,
        },
    )
    .await
    .expect("seed subscribe");
    // 1. The writer takes the GET and holds it; three unstamped live puts.
    let q = w.next_query().await;
    w.matched().await;
    w.put("a", "a-unstamped", None).await;
    w.put("c", "c-1", None).await;
    w.put("c", "c-2", None).await;
    for want in ["a-unstamped", "c-1", "c-2"] {
        let (_, _, value, stamp) = next_admitted(&mut sub).await;
        assert_eq!((value.as_str(), stamp), (want, None), "kept as it arrived");
    }
    // 2. The answer: `a` stamped T1, `b` stamped T2, and an unstamped `c`
    //    the writer held from before its live puts.
    let t1 = w.stamp(1, "a1");
    q.reply("foreign/a", "a-stamped")
        .timestamp(t1)
        .await
        .expect("reply");
    q.reply("foreign/b", "b-1")
        .timestamp(w.stamp(2, "a1"))
        .await
        .expect("reply");
    q.reply("foreign/c", "c-0").await.expect("reply");
    drop(q);
    let mut admitted = Vec::new();
    for _ in 0..3 {
        admitted.push(next_admitted(&mut sub).await);
    }
    admitted.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(
        admitted,
        [
            ("a".into(), SampleKind::Put, "a-stamped".into(), Some(t1)),
            (
                "b".into(),
                SampleKind::Put,
                "b-1".into(),
                Some(w.stamp(2, "a1"))
            ),
            ("c".into(), SampleKind::Put, "c-0".into(), None),
        ],
        "a stamp replaces an unstamped sample; an unstamped one replaces another, by arrival"
    );
    // 3. Once a key holds a stamp: unstamped puts and deletes are not kept,
    //    and a newer stamp is. A sentinel on a fresh key marks the end.
    w.put("a", "a-unstamped-2", None).await;
    writer.delete("foreign/b").await.expect("delete");
    let t3 = w.stamp(3, "a1");
    w.put("b", "b-2", Some(t3)).await;
    w.put("z", "sentinel", None).await;
    assert_eq!(
        next_admitted(&mut sub).await,
        ("b".into(), SampleKind::Put, "b-2".into(), Some(t3)),
        "the unstamped put on a and delete on b were not kept"
    );
    assert_eq!(next_admitted(&mut sub).await.2, "sentinel");
}

/// `spec/scenarios/state.md` §11, step 5: two samples of one key at one
/// time, under two ids, order as zenoh's `Timestamp` does — by the ids'
/// little-endian bytes, the least significant first — and not as the ids'
/// values: `100` (hex) is the lower id against `2`. So `2` then `100`
/// keeps `2` alone, and `100` then `2` keeps both.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s11_equal_times_order_by_the_id_as_zenoh_does() {
    let (writer, joiner) = util::peer_pair().await;
    let w = Foreign::new(&writer, &joiner).await;
    let mut sub = seed_subscribe(
        &joiner,
        "foreign/**",
        SeedPolicy {
            history: false,
            state: false,
            timeout: util::SETTLE,
        },
    )
    .await
    .expect("seed subscribe");
    w.matched().await;
    let (hi, lo) = (w.stamp(9, "2"), w.stamp(9, "100"));
    assert!(lo < hi, "zenoh's own order");
    w.put("e", "e-2", Some(hi)).await;
    w.put("e", "e-100", Some(lo)).await;
    w.put("f", "f-100", Some(lo)).await;
    w.put("f", "f-2", Some(hi)).await;
    w.put("z", "sentinel", None).await;
    let mut values = Vec::new();
    loop {
        let (_, _, value, _) = next_admitted(&mut sub).await;
        if value == "sentinel" {
            break;
        }
        values.push(value);
    }
    assert_eq!(values, ["e-2", "f-100", "f-2"], "e-100 is the lower stamp");
}

/// `spec/scenarios/state.md` §11, step 6: R6 binds a consumer; a tool's
/// seed shows what arrived. A put on the wildcard key `foreign/*` reaches
/// the seeded subscription under that key, kept apart from the concrete
/// keys it covers: `foreign/g` at T2 is still kept after the wildcard's
/// T3, which is not `g`'s.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s11_a_tools_seed_keeps_a_key_that_is_not_concrete() {
    let (writer, joiner) = util::peer_pair().await;
    let w = Foreign::new(&writer, &joiner).await;
    let mut sub = seed_subscribe(
        &joiner,
        "foreign/**",
        SeedPolicy {
            history: false,
            state: false,
            timeout: util::SETTLE,
        },
    )
    .await
    .expect("seed subscribe");
    w.matched().await;
    w.put("g", "g-1", Some(w.stamp(1, "a1"))).await;
    w.put("*", "wild", Some(w.stamp(3, "a1"))).await;
    w.put("g", "g-2", Some(w.stamp(2, "a1"))).await;
    w.put("z", "sentinel", None).await;
    let mut admitted = Vec::new();
    loop {
        let (key, _, value, _) = next_admitted(&mut sub).await;
        if value == "sentinel" {
            break;
        }
        admitted.push((key, value));
    }
    assert_eq!(
        admitted,
        [
            ("g".into(), "g-1".into()),
            ("*".into(), "wild".into()),
            ("g".into(), "g-2".into()),
        ],
        "kept under its own key, never merged with g"
    );
}

/// The payloads of a monitor's samples until `until` arrives, `until`
/// included, or until its seeded watch `seeded`'s boundary when `until` is
/// `None`.
async fn monitor_samples(
    events: &mut zenkey_fleet::EventStream,
    until: Option<&str>,
    seeded: Option<zenkey_fleet::WatchId>,
) -> Vec<String> {
    let mut seen = Vec::new();
    loop {
        let item = tokio::time::timeout(util::SETTLE, events.recv())
            .await
            .expect("an event within the settle")
            .expect("stream alive");
        match item {
            zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::Sample(s)) => {
                let value = text(&s);
                let done = until == Some(value.as_str());
                seen.push(value);
                if done {
                    return seen;
                }
            }
            zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::WatchSeeded {
                id, ..
            }) if until.is_none() && Some(id) == seeded => return seen,
            zenkey_fleet::StreamItem::Dropped(n) => {
                panic!("this fixture never outruns the broadcast ({n})")
            }
            _ => {}
        }
    }
}

/// `spec/scenarios/state.md` §7, step 4's tool (observer §4.1): a tool's
/// late joiner keeps core §4.2's merge for as long as it follows, through
/// the fleet's two seeded subscriptions. The owner, played by a writer
/// that writes its envelopes (as §10's reference does), answers the seed
/// with rev 10 at seq 10 of run 1. After the boundary it restarts under
/// run 2, and puts rev 11 at seq 0, then rev 12 at seq 1. Nothing orders
/// two runs, so the tool keeps the later arrival, rev 11, then rev 12 by
/// seq within run 2: nothing is superseded. A consumer, by core §4.3's
/// consumer rule, accepts both (`zenkey/tests/state.rs`,
/// `s7_late_joiners_across_runs`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s7_a_tools_late_joiner_keeps_the_merge_after_its_seed() {
    let policy = SeedPolicy {
        history: false,
        state: true,
        timeout: util::SETTLE,
    };
    // The fleet's seeded subscription.
    {
        let (writer, joiner) = util::peer_pair().await;
        let mut w = Enveloping::new(&writer, &joiner).await;
        let mut sub = seed_subscribe(&joiner, &format!("{ENVELOPED}/**"), policy)
            .await
            .expect("seed subscribe");
        let q = w.next_query().await;
        w.matched().await;
        q.reply(format!("{ENVELOPED}/plan"), enveloped(1, 10, "rev10"))
            .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
            .await
            .expect("reply");
        drop(q);
        let (seeded, _) = drain_seed(&mut sub).await;
        assert_eq!(seeded, ["rev10"], "the seed");
        w.put("plan", enveloped(2, 0, "rev11")).await;
        w.put("plan", enveloped(2, 1, "rev12")).await;
        assert_eq!(
            next_enveloped(&mut sub).await,
            ("plan".into(), "rev11".into(), Some((2, 0))),
            "another run: the later arrival"
        );
        assert_eq!(
            next_enveloped(&mut sub).await,
            ("plan".into(), "rev12".into(), Some((2, 1))),
            "the next seq of run 2"
        );
        assert_eq!((sub.superseded(), sub.evicted()), (0, 0));
    }
    // A monitor's seeded watch, zengui's tree.
    let (writer, joiner) = util::peer_pair().await;
    let mut w = Enveloping::new(&writer, &joiner).await;
    let monitor = zenkey_fleet::Monitor::start(&joiner, zenkey_fleet::MonitorSpec::default())
        .await
        .expect("monitor");
    let mut events = monitor.events();
    let id = monitor
        .watch_seeded(&format!("{ENVELOPED}/**"), policy)
        .await
        .expect("seeded watch");
    let q = w.next_query().await;
    w.matched().await;
    q.reply(format!("{ENVELOPED}/plan"), enveloped(1, 10, "rev10"))
        .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
        .await
        .expect("reply");
    drop(q);
    assert_eq!(
        monitor_samples(&mut events, None, Some(id)).await,
        ["rev10"],
        "the seed, before its boundary"
    );
    w.put("plan", enveloped(2, 0, "rev11")).await;
    w.put("plan", enveloped(2, 1, "rev12")).await;
    assert_eq!(
        monitor_samples(&mut events, Some("rev12"), None).await,
        ["rev11", "rev12"],
        "the later arrival across two runs, then by seq within run 2"
    );
    assert_eq!(
        (monitor.merge_superseded(), monitor.merge_evicted()),
        (0, 0)
    );
}

/// A seeded watch's merge is bounded (observer §4.1, tooling guide O6): at
/// [`zenkey_fleet::MonitorSpec::max_keys`], the key admitted least
/// recently is forgotten and counted, apart from what the merge
/// superseded. A forgotten key holds nothing, so its next sample is kept
/// whatever its stamp. A key the bound kept still refuses an older stamp.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_seeded_watchs_merge_counts_what_its_bound_forgot() {
    let (writer, joiner) = util::peer_pair().await;
    let w = Foreign::new(&writer, &joiner).await;
    let monitor = zenkey_fleet::Monitor::start(
        &joiner,
        zenkey_fleet::MonitorSpec {
            max_keys: 2,
            ..zenkey_fleet::MonitorSpec::default()
        },
    )
    .await
    .expect("monitor");
    let mut events = monitor.events();
    let id = monitor
        .watch_seeded(
            "foreign/**",
            SeedPolicy {
                history: false,
                state: false,
                timeout: util::SETTLE,
            },
        )
        .await
        .expect("seeded watch");
    assert!(
        monitor_samples(&mut events, None, Some(id))
            .await
            .is_empty()
    );
    w.matched().await;
    w.put("a", "a-5", Some(w.stamp(5, "a1"))).await;
    w.put("b", "b-5", Some(w.stamp(5, "a1"))).await;
    // A third key: `a`, admitted least recently, is forgotten.
    w.put("c", "c-5", Some(w.stamp(5, "a1"))).await;
    w.put("c", "c-4", Some(w.stamp(4, "a1"))).await;
    w.put("a", "a-1", Some(w.stamp(1, "a1"))).await;
    assert_eq!(
        monitor_samples(&mut events, Some("a-1"), None).await,
        ["a-5", "b-5", "c-5", "a-1"],
        "c-4 is older than what c holds; a, forgotten, holds nothing"
    );
    assert_eq!(
        (monitor.merge_superseded(), monitor.merge_evicted()),
        (1, 2),
        "a, then b, forgotten to admit c and a"
    );
}

/// An owner's state key under the zk2c envelope (core §7.5; #976's A2):
/// a writer with no HLC that puts each value as an envelope under
/// `zenoh/bytes` and answers GETs on its keys when the test chooses.
struct Enveloping {
    session: zenoh::Session,
    queries: tokio::sync::mpsc::UnboundedReceiver<zenoh::query::Query>,
    _queryable: zenoh::query::Queryable<()>,
}

const ENVELOPED: &str = "zk2/lab/env/m.v1/state";

/// An envelope of run `run` and seq `seq` around `record`.
fn enveloped(run: u16, seq: u32, record: &str) -> Vec<u8> {
    use zenkey_model::wire::{Envelope, Run, Seq, Stamp, Tag};
    Envelope {
        form: zenkey_model::wire::Form::Mutation,
        tag: Tag::NONE,
        stamp: Stamp {
            run: Run::U16(run),
            seq: Seq::new(seq).expect("a seq"),
        },
        record: record.as_bytes(),
    }
    .write()
}

impl Enveloping {
    async fn new(writer: &zenoh::Session, joiner: &zenoh::Session) -> Self {
        let (tx, queries) = tokio::sync::mpsc::unbounded_channel();
        let ready = format!("{ENVELOPED}/ready");
        let queryable = writer
            .declare_queryable(format!("{ENVELOPED}/**"))
            .callback({
                let ready = ready.clone();
                move |q| {
                    if q.key_expr().as_str() == ready {
                        let ready = ready.clone();
                        tokio::spawn(async move {
                            q.reply(ready, "ok").await.ok();
                        });
                    } else {
                        let _ = tx.send(q);
                    }
                }
            })
            .await
            .expect("the writer's queryable");
        let deadline = tokio::time::Instant::now() + util::SETTLE;
        loop {
            let replies = joiner
                .get(&ready)
                .timeout(Duration::from_millis(300))
                .await
                .expect("probe get");
            if replies.recv_async().await.is_ok() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "routing never converged"
            );
        }
        Enveloping {
            session: writer.clone(),
            queries,
            _queryable: queryable,
        }
    }

    /// A live put of `bytes` on `key` under `zenoh/bytes`, zenoh's default.
    async fn put(&self, key: &str, bytes: Vec<u8>) {
        self.session
            .put(format!("{ENVELOPED}/{key}"), bytes)
            .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
            .await
            .expect("the writer's put");
    }

    async fn matched(&self) {
        let publisher = self
            .session
            .declare_publisher(format!("{ENVELOPED}/probe"))
            .await
            .expect("publisher");
        let deadline = tokio::time::Instant::now() + util::SETTLE;
        while !publisher
            .matching_status()
            .await
            .expect("matching status")
            .matching()
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the subscription never reached the writer"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn next_query(&mut self) -> zenoh::query::Query {
        tokio::time::timeout(util::SETTLE, self.queries.recv())
            .await
            .expect("the GET reaches the writer")
            .expect("queryable alive")
    }
}

/// The next sample an enveloped seeded subscription admits: its key's last
/// chunk, its record (or its refusal's reason) and its in-band stamp.
async fn next_enveloped(
    sub: &mut zenkey_fleet::SeededSubscriber,
) -> (String, String, Option<(u32, u32)>) {
    loop {
        let item = tokio::time::timeout(util::SETTLE, sub.recv())
            .await
            .expect("a sample")
            .expect("stream alive");
        match item {
            SeedItem::Sample(v) => {
                let key = v.key.rsplit('/').next().unwrap_or_default().to_owned();
                let record = match v.record() {
                    Ok(b) => String::from_utf8_lossy(&b).into_owned(),
                    Err(r) => r.reason(),
                };
                let stamp = v.inband().map(|s| (s.run.value(), s.seq.get()));
                return (key, record, stamp);
            }
            SeedItem::Dropped(n) => panic!("this fixture never outruns the channel ({n})"),
            SeedItem::SeedComplete(_) => {}
        }
    }
}

/// A tool's late joiner over the zk2c envelope (core §7.5, observer §4.1;
/// #976's A2): its seed's replies and its live samples are merged by run
/// and seq. Within one run an older seq is superseded and an equal seq is
/// the same value, kept once; across two runs nothing orders them, so the
/// later arrival is kept; a refused envelope is delivered undecodable and
/// moves nothing its key holds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tools_seed_merges_enveloped_samples_by_run_and_seq() {
    let (writer, joiner) = util::peer_pair().await;
    let mut w = Enveloping::new(&writer, &joiner).await;
    let mut sub = seed_subscribe(
        &joiner,
        &format!("{ENVELOPED}/**"),
        SeedPolicy {
            history: false,
            state: true,
            timeout: util::SETTLE,
        },
    )
    .await
    .expect("seed subscribe");
    // 1. The writer holds the GET; live puts of `a` (run 1, seq 5) and `b`
    //    (run 1, seq 2).
    let q = w.next_query().await;
    w.matched().await;
    w.put("a", enveloped(1, 5, "a-5")).await;
    w.put("b", enveloped(1, 2, "b-2")).await;
    assert_eq!(
        next_enveloped(&mut sub).await,
        ("a".into(), "a-5".into(), Some((1, 5)))
    );
    assert_eq!(
        next_enveloped(&mut sub).await,
        ("b".into(), "b-2".into(), Some((1, 2)))
    );
    // 2. The answer: `a` at seq 4, older; `b` at seq 2, the same value
    //    (S2: a reply is the stored bytes); `c` new.
    for (key, bytes) in [
        ("a", enveloped(1, 4, "a-4")),
        ("b", enveloped(1, 2, "b-2")),
        ("c", enveloped(1, 7, "c-7")),
    ] {
        q.reply(format!("{ENVELOPED}/{key}"), bytes)
            .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
            .await
            .expect("reply");
    }
    drop(q);
    assert_eq!(
        next_enveloped(&mut sub).await,
        ("c".into(), "c-7".into(), Some((1, 7))),
        "a's older seq and b's equal one were not kept"
    );
    // 3. Live, after the seed: `a` under run 2 (across runs, kept), then
    //    under run 1 again (across runs once more: the later arrival, kept);
    //    a refused envelope on `a` (delivered, moving nothing); then run
    //    1's seq 3 again (the same value as held, not kept), and a
    //    sentinel.
    w.put("a", enveloped(2, 1, "a-r2")).await;
    w.put("a", enveloped(1, 3, "a-r1")).await;
    w.put("a", br#"{"legacy":true}"#.to_vec()).await;
    w.put("a", enveloped(1, 3, "a-r1")).await;
    w.put("z", enveloped(1, 1, "sentinel")).await;
    assert_eq!(
        next_enveloped(&mut sub).await,
        ("a".into(), "a-r2".into(), Some((2, 1))),
        "another run: the later arrival"
    );
    assert_eq!(
        next_enveloped(&mut sub).await,
        ("a".into(), "a-r1".into(), Some((1, 3))),
        "another run again: the later arrival, though its seq is lower"
    );
    let (key, reason, stamp) = next_enveloped(&mut sub).await;
    assert_eq!((key.as_str(), stamp), ("a", None));
    assert!(
        reason.starts_with("zk2c envelope refused: "),
        "a refused envelope is shown undecodable: {reason}"
    );
    assert_eq!(
        next_enveloped(&mut sub).await,
        ("z".into(), "sentinel".into(), Some((1, 1))),
        "a's held run 1 seq 3 stood through the refusal: its copy was not kept"
    );
    assert_eq!(sub.superseded(), 3, "a-4, b-2's reply, and a-r1's copy");
}

/// `spec/scenarios/state.md` §13, steps 1 and 2: a tool's merge of a
/// stream key orders its seqs by serial arithmetic over 24 bits (core
/// §4.3, observer §4.1), through the fleet's seeded subscription. A writer
/// of the test's own plays the owner of a plain stream, writing envelopes
/// of one run. `pose` crosses its seq's wrap, 16,777,214 to 1, and the
/// merge keeps all four: seq 0 is newer than 16,777,215. `heading` at seq 5
/// then at 8,388,613, exactly 2^23 apart: unordered, so the later arrival
/// is kept. A seq older within the run is not kept, and is counted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s13_a_stream_keys_seq_wraps_in_a_tools_merge() {
    const STREAM: &str = "zk2/lab/env/m.v1/stream";
    let (writer, joiner) = util::peer_pair().await;
    let mut sub = seed_subscribe(
        &joiner,
        &format!("{STREAM}/**"),
        SeedPolicy {
            history: false,
            state: false,
            timeout: util::SETTLE,
        },
    )
    .await
    .expect("seed subscribe");
    let probe = writer
        .declare_publisher(format!("{STREAM}/probe"))
        .await
        .expect("publisher");
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    while !probe
        .matching_status()
        .await
        .expect("matching status")
        .matching()
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the subscription never reached the writer"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let put = |key: &'static str, seq: u32, record: &'static str| {
        let writer = writer.clone();
        async move {
            writer
                .put(format!("{STREAM}/{key}"), enveloped(7, seq, record))
                .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
                .await
                .expect("the writer's put");
        }
    };
    let pose = [
        (16_777_214, "p-a"),
        (16_777_215, "p-b"),
        (0, "p-c"),
        (1, "p-d"),
    ];
    for (seq, record) in pose {
        put("pose", seq, record).await;
    }
    for (seq, record) in pose {
        assert_eq!(
            next_enveloped(&mut sub).await,
            ("pose".into(), record.into(), Some((7, seq))),
            "across the wrap, each newer than the last"
        );
    }
    put("heading", 5, "h-5").await;
    put("heading", 8_388_613, "h-far").await;
    // Older within the run: (8,388,612 − 8,388,613) mod 2^24 is above 2^23.
    put("heading", 8_388_612, "h-older").await;
    put("z", 0, "sentinel").await;
    assert_eq!(
        next_enveloped(&mut sub).await,
        ("heading".into(), "h-5".into(), Some((7, 5)))
    );
    assert_eq!(
        next_enveloped(&mut sub).await,
        ("heading".into(), "h-far".into(), Some((7, 8_388_613))),
        "exactly 2^23 apart: unordered, the later arrival kept"
    );
    assert_eq!(
        next_enveloped(&mut sub).await,
        ("z".into(), "sentinel".into(), Some((7, 0))),
        "an older seq of the run was not kept"
    );
    assert_eq!(sub.superseded(), 1, "h-older");
}

/// An owner answering in snapshots (#977's E2), the runtime's and zenctl's
/// suites' too.
#[path = "../../zenkey/tests/common/snapshot_owner.rs"]
mod snapshot_owner;

/// Drain a seeded subscriber until the boundary, keeping each sample.
async fn drain_views(
    sub: &mut zenkey_fleet::SeededSubscriber,
) -> (Vec<zenkey_fleet::SampleView>, zenkey_fleet::SeedCoverage) {
    let mut views = Vec::new();
    loop {
        match tokio::time::timeout(util::SETTLE, sub.recv())
            .await
            .expect("seed boundary")
            .expect("stream alive")
        {
            SeedItem::Sample(v) => views.push(v),
            SeedItem::Dropped(n) => panic!("{n} dropped"),
            SeedItem::SeedComplete(c) => return (views, c),
        }
    }
}

/// #977's E2: the state leg reads a snapshot's page as its entries, each
/// the per-key reply it stands for on its member's key — a live entry a
/// value with its stamp, its own run where it has one, a tombstone a
/// deletion with its stamp — and says which snapshots it heard. A member
/// read's snapshots are one page each, complete; a page whose body is
/// refused is delivered undecodable and moves nothing. A collection read's
/// three pages share one key, so under `Latest`, which the leg keeps until
/// E3, only the last arrives: the leg holds the snapshot incomplete, and
/// the seed possibly incomplete, the last page's entry delivered.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_state_leg_reads_a_snapshots_entries() {
    use snapshot_owner as owner;
    const ADDRESS: &str = "ground/fleet-mgr";
    let (a, b) = timestamping_pair().await;
    let _owner = owner::start(&a, ADDRESS).await;
    let policy = SeedPolicy {
        history: false,
        timeout: Duration::from_millis(800),
        ..SeedPolicy::default()
    };
    let members = format!("{}/*", owner::plans(ADDRESS));
    let deadline = std::time::Instant::now() + util::SETTLE;
    let (views, coverage) = loop {
        let mut sub = seed_subscribe(&b, &members, policy).await.expect("seed");
        let (views, coverage) = drain_views(&mut sub).await;
        if views.len() == 4 || std::time::Instant::now() > deadline {
            break (views, coverage);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(views.len(), 4, "{views:?}");
    let by_key: BTreeMap<&str, &zenkey_fleet::SampleView> =
        views.iter().map(|v| (v.key.as_str(), v)).collect();
    for (id, record) in [
        ("vehicle-01", Some("plan 1")),
        ("vehicle-02", Some("plan 2")),
        ("vehicle-03", None),
    ] {
        let v = by_key[owner::member(ADDRESS, id).as_str()];
        assert_eq!(v.inband(), Some(owner::stamp(id)), "{id}");
        match record {
            Some(r) => {
                assert_eq!(v.kind, SampleKind::Put, "{id}");
                assert_eq!(text(v), r, "{id}");
            }
            None => assert_eq!(v.kind, SampleKind::Delete, "{id}: a tombstone"),
        }
        assert!(
            v.timestamp.is_none(),
            "{id}: a snapshot carries no Zenoh timestamp"
        );
    }
    let broken = by_key[owner::member(ADDRESS, "broken").as_str()];
    assert!(broken.refusal().is_some(), "a refused body is undecodable");
    let leg = coverage.state.as_option().expect("the state leg ran");
    assert_eq!(leg.replies, 4);
    assert!(leg.complete, "{leg:?}");
    assert_eq!(leg.snapshots.len(), 3, "{leg:?}");
    assert!(leg.snapshots.iter().all(|s| s.complete && s.pages == [0]));
    assert!(leg.snapshots.iter().all(|s| s.run == "0005" && s.asof == 7));

    // A collection read: three pages on the snapshot key, every one read
    // under `None` (#977's E3), the snapshot complete.
    let collection = format!("{}/**", owner::plans(ADDRESS));
    let mut sub = seed_subscribe(&b, &collection, policy).await.expect("seed");
    let (views, coverage) = drain_views(&mut sub).await;
    let leg = coverage.state.as_option().expect("the state leg ran");
    assert_eq!(leg.snapshots.len(), 1, "{leg:?}");
    let s = &leg.snapshots[0];
    assert_eq!(s.key, owner::plans(ADDRESS));
    assert_eq!(s.pages, [0, 1, 2], "every page under None: {s:?}");
    assert!(s.complete && leg.complete, "{leg:?}");
    let mut keys: Vec<String> = views.iter().map(|v| v.key.clone()).collect();
    keys.sort();
    let mut entries: Vec<String> = owner::snapshot()
        .paginate(owner::PAGE_BYTES)
        .expect("pages")
        .iter()
        .flat_map(|p| p.members(&owner::plans(ADDRESS)))
        .map(|m| m.key)
        .collect();
    entries.sort();
    assert_eq!(keys, entries, "every page's entries");
}

/// #977's E2: a `.zsnap` folds a snapshot's entries into rows, each on its
/// member's key with its in-band stamp, a tombstone a deletion's row, and a
/// page whose body is refused a row of its own on its key, undecodable;
/// the header counts each entry a reply.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_zsnap_folds_a_snapshots_entries_into_rows() {
    use snapshot_owner as owner;
    const ADDRESS: &str = "ground/fleet-mgr";
    let (a, b) = timestamping_pair().await;
    let _owner = owner::start(&a, ADDRESS).await;
    let contracts = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../zenkey/tests/contracts/mission_plan.v1.toml");
    let (set, problems) = zenkey_fleet::ContractSet::load_path(&contracts);
    assert!(problems.is_empty(), "{problems:?}");
    let lens = zenkey_fleet::Lens::new("", None, &set).offline(&set);
    let spec = zenkey_fleet::SnapshotSpec {
        selectors: vec![format!("{}/*", owner::plans(ADDRESS))],
        timeout: Duration::from_millis(800),
        max_replies: 64,
    };
    let deadline = std::time::Instant::now() + util::SETTLE;
    let taken = loop {
        let t = zenkey_fleet::take_snapshot(&b, &lens, &spec)
            .await
            .expect("taken");
        assert!(
            t.incomplete.is_empty(),
            "every GET was asked: {:?}",
            t.incomplete
        );
        if t.snapshot.rows.len() == 4 || std::time::Instant::now() > deadline {
            break t;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let rows = &taken.snapshot.rows;
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert_eq!(rows[0].key, owner::member(ADDRESS, "broken"));
    assert!(
        rows[0].conformance.is_violation(),
        "{:?}",
        rows[0].conformance
    );
    let rows = &rows[1..];
    for (row, id) in rows.iter().zip(["vehicle-01", "vehicle-02", "vehicle-03"]) {
        assert_eq!(row.key, owner::member(ADDRESS, id));
        assert_eq!(
            row.inband,
            Some(zenkey_fleet::report::InbandStamp::from(owner::stamp(id))),
            "{id}"
        );
        assert_eq!(row.delete, id == "vehicle-03", "{id}");
    }
    assert_eq!(rows[0].record().as_deref(), Some(&b"plan 1"[..]));
    assert_eq!(
        taken.snapshot.header.answered, 4,
        "each entry a reply, and the refused page"
    );
}

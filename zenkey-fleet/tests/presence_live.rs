//! Live presence, per address, against live services (#614, FL1): owners
//! brought up and stopped with the runtime's own `ServiceBuilder`, read by a
//! [`PresenceFeed`] fed by a real [`Monitor`], the way zenwatch's
//! dead-man's switch and zengui's roster will read them.
//!
//! One router and client sessions, as the zk2 suites do it. The first test
//! runs the realistic two-session shape: the owners and the seed's session
//! in a namespace, the monitor on a session in none, so every live key
//! arrives prefixed and the projection strips it. The others run on one
//! session at the bus root, the liveliness GET beside the monitor's
//! liveliness subscribers (spec §8.1).

use std::path::Path;
use std::time::{Duration, SystemTime};

mod util;
use util::zk2::{T, addr, client, client_ns, config, eventually, iface, router};

use zenkey::{Implementation, Service, ServiceBuilder};
use zenkey_fleet::{
    AddressStatus, LivePresence, MemberId, Monitor, MonitorSpec, PresenceBasis, PresenceChange,
    PresenceFeed, PresenceScope, PresenceSource, PresenceTransition, SeedOutcome,
};
use zenkey_model::contract::{Contract, load_path};

use AddressStatus::{Down, Unobservable, Up};

/// A contract from the runtime's scenario set (`zenkey/tests/contracts/`).
fn scenario(name: &str) -> Contract {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../zenkey/tests/contracts/{name}.toml"));
    let l = load_path(&path);
    l.contract
        .unwrap_or_else(|| panic!("{name} does not load:\n{}", l.report))
}

/// Brings up `cfg` implementing `contract`, every resource it can expose
/// exposed (one gated on a capability not held stays absent).
async fn bring_up(session: &zenoh::Session, cfg: zenkey::ServiceConfig, c: &Contract) -> Service {
    let mut b = ServiceBuilder::new(session, cfg);
    let id = c.iface.clone();
    b.implement(Implementation::new(c.clone()))
        .expect("implement");
    for r in &c.resources {
        let _ = b.expose(&id, &zenkey::implementation::resource_name(r));
    }
    b.start().await.expect("start")
}

/// Folds the feed into `seen` until `want` holds of it, failing after
/// [`util::SETTLE`] with everything seen.
async fn until(
    feed: &mut PresenceFeed,
    seen: &mut Vec<PresenceTransition>,
    what: &str,
    want: impl Fn(&[PresenceTransition]) -> bool,
) {
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    while !want(seen) {
        match tokio::time::timeout_at(deadline, feed.next()).await {
            Ok(Some(moved)) => seen.extend(moved),
            Ok(None) => panic!("the monitor's broadcast closed"),
            Err(_) => panic!("never: {what}; saw {seen:#?}"),
        }
    }
}

fn has(seen: &[PresenceTransition], change: &PresenceChange) -> bool {
    seen.iter().any(|t| t.change == *change)
}

/// One address's instance and status moves, in order: what the brief
/// calls "says so in that order".
fn instances_and_status(seen: &[PresenceTransition], a: &str) -> Vec<PresenceChange> {
    seen.iter()
        .filter(|t| t.address.to_string() == a)
        .filter(|t| {
            matches!(
                t.change,
                PresenceChange::Status { .. }
                    | PresenceChange::InstanceAppeared { .. }
                    | PresenceChange::InstanceGone { .. }
            )
        })
        .map(|t| t.change.clone())
        .collect()
}

/// An owner starts (up), a second instance of its address joins, the first
/// stops (still up), the last stops (down): the feed, seeded from a
/// complete read and fed by a real monitor, says so in that order, every
/// live move heard as an event. The owners and the seed are in a
/// namespace and the monitor is not, so each live key is stripped.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_address_goes_up_and_down_as_its_instances_come_and_go() {
    const NS: &str = "site";
    let (_router, ep) = router(None).await;
    let first = client_ns(&ep, Some(NS)).await;
    let second = client_ns(&ep, Some(NS)).await;
    let tool = client_ns(&ep, Some(NS)).await;
    let raw = client(&ep).await;
    let ping = scenario("ping.v1");
    let h1 = addr("h1/tc");

    let scope = PresenceScope::all();
    let monitor = Monitor::start(&raw, MonitorSpec::default().with_presence(NS, &scope))
        .await
        .expect("monitor");
    let mut presence = LivePresence::new(scope).wire_namespace(NS);
    presence.track(&h1, SystemTime::now());
    let (mut feed, seeded) = PresenceFeed::open(&tool, monitor.events(), presence, T).await;
    assert_eq!(*feed.last_seed(), SeedOutcome::Complete);
    assert_eq!(feed.presence().basis(), PresenceBasis::Complete);
    assert_eq!(feed.presence().status(&h1), Down, "complete, and empty");
    let mut seen = seeded;
    assert_eq!(
        instances_and_status(&seen, "h1/tc"),
        [PresenceChange::Status {
            from: Unobservable,
            to: Down
        }]
    );

    // 1. An owner starts: up.
    let a = bring_up(&first, config("h1/tc"), &ping).await;
    until(&mut feed, &mut seen, "h1/tc up", |s| {
        has(s, &PresenceChange::Status { from: Down, to: Up })
    })
    .await;
    // 2. A second instance joins: up, with two.
    let b = bring_up(&second, config("h1/tc"), &ping).await;
    let (ia, ib) = (a.instance().clone(), b.instance().clone());
    until(&mut feed, &mut seen, "the second instance", |s| {
        has(
            s,
            &PresenceChange::InstanceAppeared {
                instance: ib.clone(),
            },
        )
    })
    .await;
    assert_eq!(feed.presence().status(&h1), Up);
    // 3. The first stops: still up.
    a.close().await.expect("close");
    until(&mut feed, &mut seen, "the first instance gone", |s| {
        has(
            s,
            &PresenceChange::InstanceGone {
                instance: ia.clone(),
            },
        )
    })
    .await;
    assert_eq!(feed.presence().status(&h1), Up, "one of two left");
    // 4. The last stops: down.
    b.close().await.expect("close");
    until(&mut feed, &mut seen, "h1/tc down", |s| {
        has(s, &PresenceChange::Status { from: Up, to: Down })
    })
    .await;

    assert_eq!(
        instances_and_status(&seen, "h1/tc"),
        [
            PresenceChange::Status {
                from: Unobservable,
                to: Down
            },
            PresenceChange::InstanceAppeared {
                instance: ia.clone()
            },
            PresenceChange::Status { from: Down, to: Up },
            PresenceChange::InstanceAppeared {
                instance: ib.clone()
            },
            PresenceChange::InstanceGone {
                instance: ia.clone()
            },
            PresenceChange::InstanceGone {
                instance: ib.clone()
            },
            PresenceChange::Status { from: Up, to: Down },
        ],
        "in that order"
    );
    assert!(
        seen[1..].iter().all(|t| t.source == PresenceSource::Event),
        "every live move was heard as it happened: {seen:#?}"
    );
    // Each instance's interface token came and went too, under its own
    // fingerprint prefix.
    for i in [&ia, &ib] {
        for appeared in [true, false] {
            assert!(
                seen.iter().any(|t| match &t.change {
                    PresenceChange::InterfaceAppeared {
                        instance, iface: f, ..
                    } if appeared => instance == i && *f == iface("ping.v1"),
                    PresenceChange::InterfaceGone {
                        instance, iface: f, ..
                    } if !appeared => instance == i && *f == iface("ping.v1"),
                    _ => false,
                }),
                "{i}'s interface token ({appeared}): {seen:#?}"
            );
        }
    }
    let rec = feed.presence().address(&h1).expect("tracked");
    assert!(!rec.holds_any());
    assert_eq!(feed.presence().ignored().total(), 0, "nothing but presence");
    drop(feed);
    monitor.shutdown().await.expect("monitor shutdown");
}

/// A member's token cycled make-before-break is a handover on a live bus,
/// and the older epoch's delete after it moves nothing; withdrawn, the
/// member is gone. One session at the bus root holds the monitor's
/// liveliness subscribers and makes the seed's liveliness GET (§8.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_member_epoch_handover_is_no_gap_on_a_live_bus() {
    let (_router, ep) = router(None).await;
    let owner = client(&ep).await;
    let tool = client(&ep).await;
    let nav = iface("nav.v2");
    let scope = PresenceScope::service(&addr("p3/nav"));
    let monitor = Monitor::start(&tool, MonitorSpec::default().with_presence("", &scope))
        .await
        .expect("monitor");
    let mut svc = bring_up(&owner, config("p3/nav"), &scenario("nav.v2")).await;
    let (mut feed, seeded) =
        PresenceFeed::open(&tool, monitor.events(), LivePresence::new(scope), T).await;
    let mut seen = seeded;
    until(&mut feed, &mut seen, "p3/nav up", |s| {
        s.iter().any(|t| {
            matches!(
                t.change,
                PresenceChange::Status {
                    to: AddressStatus::Up,
                    ..
                }
            )
        })
    })
    .await;

    let a = MemberId {
        iface: nav.clone(),
        member: "a".into(),
    };
    let first = svc.declare_member(&nav, "a").await.expect("a member");
    until(&mut feed, &mut seen, "member a", |s| {
        has(
            s,
            &PresenceChange::MemberAppeared {
                member: a.clone(),
                epoch: first.clone(),
            },
        )
    })
    .await;
    let second = svc.cycle_member(&nav, "a").await.expect("a cycle");
    until(&mut feed, &mut seen, "a's handover", |s| {
        has(
            s,
            &PresenceChange::MemberHandover {
                member: a.clone(),
                from: first.clone(),
                to: second.clone(),
            },
        )
    })
    .await;
    // The older epoch's delete folds without a transition: wait for the
    // record to hold the newer epoch alone.
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    loop {
        let rec = feed.presence().address(&addr("p3/nav")).expect("a record");
        if rec.members.get(&a).is_some_and(|e| *e == [second.clone()]) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the older epoch never went: {rec:#?}"
        );
        if let Ok(Some(moved)) = tokio::time::timeout(Duration::from_millis(50), feed.next()).await
        {
            seen.extend(moved);
        }
    }
    assert!(
        !seen
            .iter()
            .any(|t| matches!(t.change, PresenceChange::MemberGone { .. })),
        "a handover is no gap: {seen:#?}"
    );

    svc.remove_member(&nav, "a").await.expect("withdrawn");
    until(&mut feed, &mut seen, "a gone", |s| {
        has(
            s,
            &PresenceChange::MemberGone {
                member: a.clone(),
                epoch: second.clone(),
            },
        )
    })
    .await;
    assert_eq!(feed.presence().status(&addr("p3/nav")), Up);
    svc.close().await.expect("close");
    drop(feed);
    monitor.shutdown().await.expect("monitor shutdown");
}

/// A `Dropped(n)` makes the address unobservable at once, and the re-seed
/// after it finds what the lost events said: an instance gone, learned from
/// the seed, and the address down. The loss is forced by overflowing the
/// monitor's broadcast with keys that are not presence; the stop it hides
/// is real.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_loss_is_unobservable_until_the_reseed_finds_what_it_hid() {
    let (_router, ep) = router(None).await;
    let owner = client(&ep).await;
    let tool = client(&ep).await;
    let h1 = addr("h1/tc");
    let scope = PresenceScope::all();
    let spec = MonitorSpec {
        capacity: 2,
        stats_tick: Duration::from_secs(3600),
        ..MonitorSpec::default()
    };
    let monitor = Monitor::start(&tool, spec.with_presence("", &scope))
        .await
        .expect("monitor");
    let a = bring_up(&owner, config("h1/tc"), &scenario("ping.v1")).await;
    let ia = a.instance().clone();
    eventually("h1/tc's instance token at the router", || async {
        zenkey_fleet::liveliness_read(&tool, "zk2/h1/tc/@zk/instance/*", T)
            .await
            .is_ok_and(|r| r.keys.len() == 1)
    })
    .await;
    let (mut feed, seeded) =
        PresenceFeed::open(&tool, monitor.events(), LivePresence::new(scope), T).await;
    let mut seen = seeded;
    assert_eq!(feed.presence().status(&h1), Up);

    // The owner stops, and its deletes are lost in an overflow.
    a.close().await.expect("close");
    eventually("h1/tc gone from the router", || async {
        zenkey_fleet::liveliness_read(&tool, "zk2/h1/tc/@zk/instance/*", T)
            .await
            .is_ok_and(|r| r.keys.is_empty() && r.complete)
    })
    .await;
    for i in 0..16 {
        monitor.core().node_event(format!("demo/flood/{i}"), true);
    }
    until(&mut feed, &mut seen, "h1/tc down again", |s| {
        s.iter().any(|t| {
            t.change
                == PresenceChange::Status {
                    from: Unobservable,
                    to: Down,
                }
        })
    })
    .await;
    let moved: Vec<(PresenceSource, PresenceChange)> = seen
        .iter()
        .filter(|t| {
            matches!(
                t.change,
                PresenceChange::Status { .. } | PresenceChange::InstanceGone { .. }
            )
        })
        .map(|t| (t.source, t.change.clone()))
        .collect();
    let tail = &moved[moved.len() - 3..];
    assert_eq!(
        tail,
        [
            (
                PresenceSource::Dropped,
                PresenceChange::Status {
                    from: Up,
                    to: Unobservable
                }
            ),
            (
                PresenceSource::Seed,
                PresenceChange::InstanceGone { instance: ia }
            ),
            (
                PresenceSource::Seed,
                PresenceChange::Status {
                    from: Unobservable,
                    to: Down
                }
            ),
        ],
        "{seen:#?}"
    );
    assert_eq!(*feed.last_seed(), SeedOutcome::Complete);
    assert_eq!(feed.presence().basis(), PresenceBasis::Complete);
    drop(feed);
    monitor.shutdown().await.expect("monitor shutdown");
}

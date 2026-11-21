//! The fleet's live readers hear an owner heartbeat (core §7.5;
//! `freshness.v2` §2.5; #1110's H2c): the health window, through the
//! runtime's consumer, and the alarm feed's own subscriber on the class
//! key. No owner sends one yet, so a task puts them from the owner's
//! session, through the model's writer, until the reader has read one.
//!
//! Setup: R a router; the owner `site-a/rf0` implements `health.v1` and
//! `alarms.v1` (one alarm firing) under a catalog that lists both; a tool
//! on another client, given that catalog or none.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

mod util;
use util::zk2::{T, addr, client, config, eventually, router, watch_of};

use zenkey::{Service, ServiceBuilder};
use zenkey_fleet::Catalogs;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::catalog::Catalog;
use zenkey_model::contract::Contract;
use zenkey_model::freshness::Observation;
use zenkey_model::heartbeat::{
    Class, Digest, Group, Layout, Members, OwnerHeartbeat, Section, Time, owner_key,
};
use zenkey_model::wire::{Seq, Stamp};

const ADDRESS: &str = "site-a/rf0";

/// The deployment's catalog: `health.v1` at index 0, `alarms.v1` at 1.
fn catalog() -> Catalog {
    let mut c = Catalog::new(0).unwrap();
    for k in [
        zenkey::health::implementation().contract().clone(),
        zenkey::alarms::implementation().contract().clone(),
    ] {
        c.push(k.iface.clone(), Fingerprint::of(&k)).unwrap();
    }
    c
}

async fn owner(owners: &zenoh::Session) -> (Service, String) {
    let mut b = ServiceBuilder::new(owners, config(ADDRESS));
    b.catalog(catalog()).expect("a catalog");
    let h = b.health().await.expect("health.v1");
    let a = b
        .alarms(zenkey::alarms::Startup::default())
        .await
        .expect("alarms.v1");
    h.set_status(zenkey::health::Level::Ok, "serving")
        .await
        .expect("a status");
    let svc = b.start().await.expect("started");
    let raised = a
        .raise(zenkey::alarms::Condition::new(
            "link_down",
            zenkey_model::alarms::Severity::Warning,
            "link down",
        ))
        .await
        .expect("raised");
    (svc, raised.alert_key)
}

/// The one member the owner holds of `resource`, as its S4 GET answers:
/// its key and its in-band stamp.
async fn member(tool: &zenoh::Session, contract: &Contract, resource: &str) -> (String, Stamp) {
    let c = zenkey::consumer::Consumer::for_tool(
        tool,
        Arc::new(contract.clone()),
        &[ADDRESS],
        &Default::default(),
    )
    .expect("a consumer");
    let found = Arc::new(std::sync::Mutex::new(None));
    eventually("the owner answers its member", || async {
        let answer = c.get_answer(resource, None, T).await.ok();
        let one = answer.and_then(|a| a.inband.into_iter().next());
        *found.lock().unwrap() = one;
        found.lock().unwrap().is_some()
    })
    .await;
    let one = found.lock().unwrap().take();
    one.expect("its member")
}

/// Puts an owner heartbeat on the `host` key every 100 ms, each holding
/// one group at `index` whose section vouches for `key` under `stamp`,
/// until the handle is aborted.
fn pump(
    owners: &zenoh::Session,
    index: u16,
    contract: &Contract,
    resource: &'static str,
    key: String,
    stamp: Stamp,
    templated: bool,
) -> tokio::task::JoinHandle<()> {
    let layout = Layout::of(contract);
    let sections: Vec<Option<Section>> = layout
        .collections
        .iter()
        .map(|c| {
            (c.resource() == resource).then(|| Section {
                digest: Digest::of([(key.as_str(), stamp)]),
                members: if templated {
                    Members::templated(1, [0])
                } else {
                    Members::One { vouched: true }
                },
            })
        })
        .collect();
    let group = Group::of(index, &layout, &sections).unwrap();
    let owners = owners.clone();
    let to = owner_key("site-a", "rf0", Class::Host);
    tokio::spawn(async move {
        // Bounded: far more puts than any reader here waits for.
        for seq in 0u32..10_000 {
            let bytes = OwnerHeartbeat {
                // A known time: the health reader's clock question reads it.
                time: Time::At { secs: 5, ms: 0 },
                groups: vec![group.clone()],
            }
            .envelope(Stamp {
                run: stamp.run,
                seq: Seq::new(seq).unwrap(),
            })
            .unwrap();
            owners.put(&to, bytes).await.expect("a put");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
}

/// The health window hears the owner heartbeat's group for `health.v1`
/// as a heartbeat read in step, given the catalog its index is read
/// through; given none it reads no group, and hears no heartbeat.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_health_window_reads_an_owner_heartbeats_group() {
    use zenkey_model::health::Heard;
    let (_r, ep) = router(None).await;
    let (owners, tool) = (client(&ep).await, client(&ep).await);
    let (svc, _) = owner(&owners).await;
    let contract = zenkey::health::implementation().contract().clone();
    let (key, stamp) = member(&tool, &contract, zenkey::health::STATUS).await;
    let pumping = pump(
        &owners,
        0,
        &contract,
        zenkey::health::STATUS,
        key,
        stamp,
        false,
    );
    let heard = |catalogs: Catalogs| {
        let tool = tool.clone();
        async move {
            let obs = zenkey_fleet::bus::health::observe(
                &tool,
                "",
                zenkey_fleet::HealthTarget::One(addr(ADDRESS)),
                zenkey_fleet::HealthSpec {
                    timeout: T,
                    window: Some(Duration::from_secs(2)),
                    grace: Duration::from_millis(200),
                    clocks_synced: true,
                    face: None,
                    catalogs,
                },
                None,
                |_| {},
            )
            .await;
            let window = obs.window.expect("a window ran").expect("it opened");
            window
                .heard
                .get(&addr(ADDRESS))
                .map(|h| h.sequence.clone())
                .unwrap_or_default()
        }
    };
    let given = heard(Catalogs::new(vec![catalog()])).await;
    assert!(
        given.contains(&Heard::Heartbeat),
        "the group read in step, its time known: {given:?}"
    );
    let none = heard(Catalogs::default()).await;
    assert!(
        !none.contains(&Heard::Heartbeat) && !none.contains(&Heard::HeartbeatHeld),
        "given no catalog no group is read: {none:?}"
    );
    pumping.abort();
    svc.close().await.expect("closed");
}

/// The alarm feed subscribes the owners' class key beside the heartbeat's
/// (its second heartbeat subscriber), and folds the group for `alarms.v1`
/// into its book: the alarm, put before the feed opened, is confirmed by
/// the owner heartbeat alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_alarm_feed_reads_an_owner_heartbeats_group() {
    let (_r, ep) = router(None).await;
    let (owners, tool, watch) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let (svc, _) = owner(&owners).await;
    let contract = zenkey::alarms::implementation().contract().clone();
    let (key, stamp) = member(&tool, &contract, zenkey::alarms::ALARMS).await;
    let mut spec = zenkey_fleet::AlarmSpec::new(T);
    spec.catalogs = Catalogs::new(vec![catalog()]);
    let scope = zenkey_fleet::AlarmScope::of(&addr(ADDRESS));
    assert_eq!(
        scope.owner_heartbeat_selector(),
        "zk2/site-a/rf0/freshness.v2/stream/heartbeats/host"
    );
    let (mut feed, _) =
        zenkey_fleet::AlarmFeed::open(&tool, Some(&watch_of(&watch, "")), scope, spec, true)
            .await
            .expect("the reader opens");
    let confirmed = |feed: &zenkey_fleet::AlarmFeed| {
        feed.book()
            .reading(&key, Instant::now(), SystemTime::now())
            .is_some_and(|r| {
                r.observations.iter().any(|o| {
                    matches!(
                        o,
                        Observation::Subscribed {
                            last: Some(_),
                            complete: true,
                            ..
                        }
                    )
                })
            })
    };
    assert!(
        !confirmed(&feed),
        "nothing was delivered on the subscription, and no heartbeat heard yet"
    );
    let pumping = pump(
        &owners,
        1,
        &contract,
        zenkey::alarms::ALARMS,
        key.clone(),
        stamp,
        true,
    );
    let deadline = Instant::now() + util::SETTLE;
    while !confirmed(&feed) {
        assert!(
            Instant::now() < deadline,
            "no owner heartbeat confirmed the alarm: {} read",
            feed.book().heartbeats_read()
        );
        let _ = tokio::time::timeout(Duration::from_millis(250), feed.next()).await;
    }
    assert!(feed.book().heartbeats_read() >= 1);
    pumping.abort();
    svc.close().await.expect("closed");
}

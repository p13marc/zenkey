//! #1087: the health, alarms and `check conform` readers date a snapshot's
//! confirmations under the epoch of the catalog an owner's descriptor
//! names, read through the catalogs given (`--catalog`, core §8.6;
//! `freshness.v2` §2.6; `bus::query::EntryEpochs`). Each test runs an
//! owner under zenoh-modem's catalog (`catalog.json`'s head, `29bf7d47`
//! since modem.v3 3.4), whose epoch is
//! 2026-01-01T00:00:00Z, confirmed: given that catalog, the reader dates the
//! confirmation at about now; given none, it leaves it undated, never
//! dated under 0.

use std::path::Path;
use std::time::{Duration, SystemTime};

mod util;
use util::zk2::{T, client, config, eventually, router, watch_of};

use zenkey::{Service, ServiceBuilder};
use zenkey_fleet::{Catalogs, DeploymentCatalog};
use zenkey_model::grammar::Addr;

const ADDRESS: &str = "site-a/rf0";

fn modem_catalog() -> DeploymentCatalog {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2/zenoh-modem/catalog.json");
    Catalogs::load(&path).expect("zenoh-modem's catalog")
}

fn addr() -> Addr {
    ADDRESS.parse().expect("an address")
}

/// The catalogs a reader is given: zenoh-modem's, or none.
fn given(modem: bool) -> Catalogs {
    if modem {
        Catalogs::new(vec![modem_catalog()])
    } else {
        Catalogs::default()
    }
}

/// `time` is within a minute of this host's clock: dated under the
/// catalog's epoch, not 0 (which would put it in 1970).
fn about_now(time: SystemTime) -> bool {
    let now = SystemTime::now();
    let gap = now
        .duration_since(time)
        .or_else(|_| time.duration_since(now))
        .expect("a gap");
    gap < Duration::from_secs(60)
}

/// Seconds since the Unix epoch of an RFC 3339 UTC time, to the second.
fn unix_of(rfc3339: &str) -> i64 {
    let n = |r: std::ops::Range<usize>| rfc3339[r].parse::<i64>().expect("a number");
    let (y, m, d) = (n(0..4), n(5..7), n(8..10));
    let (hh, mm, ss) = (n(11..13), n(14..16), n(17..19));
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + hh * 3_600 + mm * 60 + ss
}

fn about_now_text(rfc3339: &str) -> bool {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("after 1970")
        .as_secs();
    (unix_of(rfc3339) - i64::try_from(now).expect("a time")).abs() < 60
}

/// An owner under zenoh-modem's catalog, its descriptor naming its head,
/// implementing `health.v1` (status `OK`) and `alarms.v1` (one alarm
/// firing), both on revision 1.1, which `freshness.v2` confirms.
async fn modem_owner(owners: &zenoh::Session) -> (Service, String) {
    let mut b = ServiceBuilder::new(owners, config(ADDRESS));
    b.catalog(modem_catalog()).expect("a catalog");
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

fn health_spec(catalogs: Catalogs) -> zenkey_fleet::HealthSpec {
    zenkey_fleet::HealthSpec {
        timeout: T,
        window: None,
        grace: Duration::from_millis(200),
        clocks_synced: true,
        face: None,
        catalogs,
    }
}

/// `zenctl health --catalog`: the status's confirmation dated at about
/// now, and the service healthy; with no catalog, undated.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn health_dates_a_modem_owners_confirmation() {
    let (_r, ep) = router(None).await;
    let (owners, tool) = (client(&ep).await, client(&ep).await);
    let (svc, _) = modem_owner(&owners).await;
    let status = |modem: bool| {
        let tool = tool.clone();
        async move {
            let obs = zenkey_fleet::bus::health::observe(
                &tool,
                "",
                zenkey_fleet::HealthTarget::One(addr()),
                health_spec(given(modem)),
                None,
                |_| {},
            )
            .await;
            obs.first
                .ok()
                .and_then(|g| g.services.get(&addr()).cloned())
                .and_then(|s| s.status)
        }
    };
    eventually("the owner answers its status", || async {
        status(true).await.is_some_and(|s| s.stamp.is_some())
    })
    .await;
    let dated = status(true).await.expect("a status");
    let stamp = dated.stamp.expect("dated under the catalog's epoch");
    assert!(about_now(stamp.time), "{stamp:?}");
    let undated = status(false).await.expect("a status");
    assert_eq!(
        undated.stamp, None,
        "no catalog given: undated, never under 0"
    );
    let report = zenkey_fleet::run_health(
        &tool,
        "",
        zenkey_fleet::HealthTarget::One(addr()),
        health_spec(given(true)),
        |_| {},
    )
    .await;
    let row = report
        .services
        .iter()
        .find(|r| r.address == ADDRESS)
        .expect("its row");
    assert_eq!(
        row.verdict,
        zenkey_fleet::report::HealthVerdict::Healthy,
        "confirmed, with its time: {row:?}"
    );
    svc.close().await.expect("closed");
}

/// `zenctl alarms --catalog`: the alarm's confirmation dated at about now,
/// and it reads firing; with no catalog, undated.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn alarms_date_a_modem_owners_confirmation() {
    let (_r, ep) = router(None).await;
    let (owners, tool) = (client(&ep).await, client(&ep).await);
    let (svc, key) = modem_owner(&owners).await;
    let read = |modem: bool| {
        let tool = tool.clone();
        async move {
            let mut spec = zenkey_fleet::AlarmSpec::new(T);
            spec.clocks_synced = true;
            spec.archives = false;
            spec.catalogs = given(modem);
            zenkey_fleet::run_alarms(
                &tool,
                None,
                "",
                zenkey_fleet::AlarmScope::of(&addr()),
                spec,
                zenkey_model::alarms::Severity::Warning,
                || {},
            )
            .await
        }
    };
    eventually("the owner answers its alarm", || async {
        read(true)
            .await
            .alarms
            .iter()
            .any(|r| r.alarm.alert_key == key)
    })
    .await;
    let report = read(true).await;
    let row = report
        .alarms
        .iter()
        .find(|r| r.alarm.alert_key == key)
        .expect("its row");
    let stamp = row
        .latest
        .as_ref()
        .and_then(|l| l.stamp.as_ref())
        .expect("dated under the catalog's epoch");
    assert!(about_now_text(&stamp.time), "{stamp:?}");
    assert_eq!(
        row.firing.answer,
        zenkey_fleet::report::AlarmAnswerToken::Yes,
        "confirmed, with its time: {row:?}"
    );
    let report = read(false).await;
    let row = report
        .alarms
        .iter()
        .find(|r| r.alarm.alert_key == key)
        .expect("its row");
    assert_eq!(
        row.latest.as_ref().and_then(|l| l.stamp.as_ref()),
        None,
        "no catalog given: undated, never under 0"
    );
    svc.close().await.expect("closed");
}

/// `check conform --catalog`: the alarms GET's entry and the health
/// reading's status dated at about now; with no catalog, undated.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn conform_dates_a_modem_owners_confirmations() {
    let (_r, ep) = router(None).await;
    let (owners, tool, raw) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let (svc, key) = modem_owner(&owners).await;
    let bus = zenkey_fleet::DoctorBus {
        session: tool.clone(),
        raw,
        namespace: String::new(),
        watch: None,
    };
    let spec = zenkey_fleet::ConformSpec {
        timeout: T,
        window: Duration::from_millis(500),
        call_all: false,
        seed: 0,
        trust_admin: false,
        calls_granted: false,
        clocks_synced: true,
        lease: zenkey_model::budget::DEFAULT_LEASE,
        wait_before_close: zenkey_model::budget::DEFAULT_WAIT_BEFORE_CLOSE,
        skip: zenkey_fleet::bus::conform::Skipped::default(),
    };
    let observe = |modem: bool| {
        let bus = &bus;
        async move {
            zenkey_fleet::bus::conform::observe(
                bus,
                &zenkey_fleet::ContractSet::new(),
                &given(modem),
                addr(),
                zenkey::health::iface(),
                None,
                spec,
            )
            .await
        }
    };
    let alarm_stamp = |obs: &zenkey_fleet::bus::conform::ConformObservation| {
        obs.alarms
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .and_then(|s| s.replies.iter().find(|r| r.key.ends_with(&key)))
            .and_then(|r| r.stamp)
    };
    let status_stamp = |obs: &zenkey_fleet::bus::conform::ConformObservation| {
        obs.health
            .as_ref()
            .and_then(|h| h.first.as_ref().ok())
            .and_then(|g| g.services.get(&addr()).cloned())
            .and_then(|s| s.status)
            .and_then(|s| s.stamp)
    };
    let dated = observe(true).await;
    let alarm = alarm_stamp(&dated).expect("the alarm dated under the catalog's epoch");
    assert!(about_now(alarm.get_time().to_system_time()), "{alarm:?}");
    let status = status_stamp(&dated).expect("the status dated under the catalog's epoch");
    assert!(about_now(status.time), "{status:?}");
    let undated = observe(false).await;
    assert!(
        alarm_stamp(&undated).is_none() && status_stamp(&undated).is_none(),
        "no catalog given: undated, never under 0"
    );
    svc.close().await.expect("closed");
}

/// The watchdog's `alert-firing` (`zenctl watchdog --catalog`, `record --on`,
/// zenwatch's `catalog`): the same alarms reader, so a modem owner's alarm,
/// confirmed and dated under the catalog, fires; with no catalog its
/// confirmation is undated, and the rule never reads it firing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_watchdogs_alert_firing_dates_a_modem_owners_confirmation() {
    use zenkey_fleet::Sipper as _;
    use zenkey_fleet::judge::condition::{Condition, WatchdogSpec, watchdog};
    let (_r, ep) = router(None).await;
    let (owners, tool, watch) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let (svc, _) = modem_owner(&owners).await;
    let rule = format!("alert-firing zk2/{ADDRESS}/** warning");
    let bus = zenkey_fleet::DoctorBus {
        session: tool.clone(),
        raw: tool.clone(),
        namespace: String::new(),
        watch: Some(watch_of(&watch, "")),
    };
    let store = zenkey_fleet::BundleStore::new(T);
    let contracts = zenkey_fleet::ContractSet::new();
    let states = |modem: bool| {
        let (bus, store, contracts, rule) = (&bus, &store, &contracts, rule.clone());
        async move {
            let spec = WatchdogSpec {
                rules: vec![Condition::parse(&rule).expect("rule")],
                tick: Duration::from_millis(300),
                ticks: Some(3),
                timeout: T,
                clocks_synced: true,
                catalogs: given(modem),
            };
            let mut run = watchdog(bus, store, contracts, &spec).pin();
            let mut to = Vec::new();
            while let Some(t) = run.sip().await {
                to.push(t.to);
            }
            run.await.expect("a run");
            to
        }
    };
    eventually("the alarm fires under the catalog", || async {
        states(true)
            .await
            .contains(&zenkey_fleet::report::CondState::Firing)
    })
    .await;
    let undated = states(false).await;
    assert!(
        !undated.contains(&zenkey_fleet::report::CondState::Firing),
        "no catalog given: undated, never read firing: {undated:?}"
    );
    svc.close().await.expect("closed");
}

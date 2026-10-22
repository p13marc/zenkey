//! `alerts <SEL>` and `alert-firing` against live owners (#800, AD): an
//! in-process router, an owner of `alarms.v1` brought up with the
//! runtime's own `ServiceBuilder::alarms` in a deployment namespace, and
//! the daemon on its two sessions, its sinks the dry-run seam (capturing).
//!
//! v1's suite (`tests/alerts.rs` before FL2) ported to zk2: a raise is one
//! firing per sink, the document's severity and labels lifted; a refresh
//! is nothing; an escalation is news; the resolve is one `resolved`; a
//! re-raise fires again; an alarm already firing when the daemon starts is
//! told, and its resolve delivered. New with alarms.v1: an owner that goes
//! away leaves its alarm unobservable, last-known — never resolved, never
//! ok; a restart with a state file is no news; a lost reading (a router's
//! restart) is said once by the observer; `alert-firing` runs as zenctl's
//! watchdog runs it; and the daemon publishes the alarm and its `alarms`
//! check.

mod util;

use std::collections::BTreeMap;
use std::time::Duration;

use util::engine::{self, Capture, Options};
use util::zk2;
use zenkey::alarms::{Alarms, Condition, Severity, Startup};
use zenkey::{Service, ServiceBuilder};
use zenkey_fleet::CondState;
use zenkey_fleet::report::{AlarmAnswerToken, AlarmChange};
use zenwatch::config::RuleConfig;
use zenwatch::render::RenderSource;
use zenwatch::sinks::{NoticeKind, Notification, Outgoing, Sink};

const NS: &str = "site";
const OWNER: &str = "lab/svc";
/// A's alert key (alarms.v1 §2.2).
const A_KEY: &str = "a659f813308ad1da";
const RULE: &str = "alerts zk2/lab/*/alarms.v1/state/alarms/*";

fn a_key() -> String {
    format!("zk2/{OWNER}/alarms.v1/state/alarms/{A_KEY}")
}

/// The alarm A of the scenarios' conventions at `severity`, its summary
/// `summary`.
fn alarm_a(severity: Severity, summary: &str) -> Condition {
    Condition::new("link_down", severity, summary)
        .label("peer", "r2")
        .label("port", "eth0")
}

/// An owner of `alarms.v1` at [`OWNER`] through the runtime, started.
async fn owner(session: &zenoh::Session) -> (Service, Alarms) {
    let mut b = ServiceBuilder::new(session, zk2::config(OWNER));
    let a = b.alarms(Startup::default()).await.expect("alarms.v1");
    (b.start().await.expect("start"), a)
}

/// The owner's session sees the daemon's subscription to A's key, so its
/// first put of A, on a publisher declared at the raise, is not filtered
/// out before it (zenoh's write-side filtering).
async fn seen_by(owners: &zenoh::Session) {
    let p = owners
        .declare_publisher(a_key())
        .await
        .expect("a publisher");
    zk2::eventually("the daemon's subscription reaches the owner", || async {
        p.matching_status().await.is_ok_and(|m| m.matching())
    })
    .await;
}

/// The `alerts` rule, `team=infra`, to `sinks`, no severity of its own.
fn alerts_rule(name: &str, sinks: &[&str]) -> RuleConfig {
    RuleConfig {
        name: name.into(),
        rule: RULE.into(),
        severity: None,
        labels: BTreeMap::from([("team".to_string(), "infra".to_string())]),
        sinks: sinks.iter().map(|s| s.to_string()).collect(),
        for_s: None,
    }
}

/// What a capture holds about A, in order.
fn of_a(got: &[Outgoing], rule: &str) -> Vec<Notification> {
    let id = format!("{rule}:{}", a_key());
    got.iter()
        .map(|o| o.notification.clone())
        .filter(|n| n.id == id)
        .collect()
}

/// Until the capture holds `n` notifications about A.
async fn about_a(capture: &Capture, rule: &str, n: usize) -> Vec<Notification> {
    let got = engine::until(capture, &format!("{n} notification(s) about A"), |s| {
        of_a(s, rule).len() >= n
    })
    .await;
    of_a(&got, rule)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_raise_fires_once_per_sink_a_refresh_is_nothing_an_escalation_is_news_and_a_resolve_resolves()
 {
    let (_router, ep) = zk2::router().await;
    let owners = zk2::client_ns(&ep, Some(NS)).await;
    let tool = zk2::client_ns(&ep, Some(NS)).await;
    let (_svc, alarms) = owner(&owners).await;
    zk2::tokens(&tool, &format!("zk2/{OWNER}/@zk/instance/*"), 1).await;

    let (ops, mail) = (Capture::default(), Capture::default());
    let running = engine::start(
        zk2::daemon_bus(&ep, NS).await,
        vec![alerts_rule("fleet-alerts", &["ops", "mail"])],
        ops.clone(),
        Options {
            extra_sinks: vec![Sink::capturing("mail", mail.clone())],
            ..Options::default()
        },
    )
    .await;
    seen_by(&owners).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(
        of_a(&ops.lock().unwrap(), "fleet-alerts").is_empty(),
        "nothing fires at the first look"
    );

    // 1. A raise: one firing per sink, the document's fields lifted.
    alarms
        .raise(alarm_a(Severity::Warning, "link to r2 down on eth0"))
        .await
        .expect("A raised");
    for sink in [&ops, &mail] {
        let got = about_a(sink, "fleet-alerts", 1).await;
        let n = &got[0];
        assert_eq!(n.kind, NoticeKind::Alert);
        assert_eq!(n.rule_kind, "alerts");
        assert_eq!((n.state, n.prior), (CondState::Firing, None));
        assert_eq!(n.severity, "warning", "the document's severity");
        assert_eq!(n.title, "warning fleet-alerts firing");
        assert_eq!(n.labels.get("team").map(String::as_str), Some("infra"));
        assert_eq!(n.labels.get("peer").map(String::as_str), Some("r2"));
        assert_eq!(n.labels.get("port").map(String::as_str), Some("eth0"));
        assert_eq!(n.rendering, RenderSource::Schema);
        assert!(
            n.evidence
                .contains("raised at warning — link to r2 down on eth0"),
            "{}",
            n.evidence
        );
        assert!(n.message.contains(&format!("key: {}", a_key())));
        assert!(n.message.contains("address: lab/svc"), "{}", n.message);
        assert!(n.message.contains("payload: {"), "{}", n.message);
        let t = n.alert.as_ref().expect("the alarm's change");
        assert_eq!(t.change, AlarmChange::Raised);
        assert_eq!(t.firing.answer, AlarmAnswerToken::Yes);
        assert_eq!(t.alarm.alert_key, A_KEY);
        assert_eq!(
            (t.alarm.system.as_str(), t.alarm.service.as_str()),
            ("lab", "svc")
        );
        let wire = serde_json::to_value(n).expect("serializes");
        assert_eq!(wire["alert"]["change"], "raised");
        assert_eq!(wire["alert"]["document"]["state"], "firing");
    }
    assert_eq!(
        ops.lock().unwrap()[0].sinks,
        vec!["ops".to_string(), "mail".to_string()]
    );

    // 2. A refresh (the summary moves at one severity) is no transition.
    alarms
        .raise(alarm_a(
            Severity::Warning,
            "link to r2 down on eth0, flapping",
        ))
        .await
        .expect("A refreshed");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(of_a(&ops.lock().unwrap(), "fleet-alerts").len(), 1);
    assert_eq!(of_a(&mail.lock().unwrap(), "fleet-alerts").len(), 1);

    // 3. An escalation is news, in its own words.
    alarms
        .raise(alarm_a(Severity::Critical, "link to r2 down on eth0"))
        .await
        .expect("A escalated");
    let got = about_a(&ops, "fleet-alerts", 2).await;
    let n = &got[1];
    assert_eq!((n.kind, n.state), (NoticeKind::Alert, CondState::Firing));
    assert_eq!(n.severity, "critical");
    assert!(
        n.evidence.contains("escalated from warning to critical"),
        "{}",
        n.evidence
    );
    assert_eq!(
        n.alert.as_ref().map(|t| t.change),
        Some(AlarmChange::Escalated)
    );

    // 4. The resolve: one `resolved`; its delete says nothing more.
    alarms.resolve(A_KEY).await.expect("A resolved");
    let got = about_a(&ops, "fleet-alerts", 3).await;
    let n = &got[2];
    assert_eq!(n.kind, NoticeKind::Resolved, "firing → ok, said as such");
    assert_eq!((n.state, n.prior), (CondState::Ok, Some(CondState::Firing)));
    assert!(n.evidence.contains(" resolved — "), "{}", n.evidence);

    // 5. A re-raise fires again.
    alarms
        .raise(alarm_a(Severity::Warning, "link to r2 down on eth0"))
        .await
        .expect("A raised again");
    let got = about_a(&ops, "fleet-alerts", 4).await;
    assert_eq!(
        (got[3].kind, got[3].state, got[3].prior),
        (NoticeKind::Alert, CondState::Firing, Some(CondState::Ok))
    );
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(
        of_a(&mail.lock().unwrap(), "fleet-alerts").len(),
        4,
        "the same four on every sink"
    );

    let summary = running.stop().await;
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.alarms_lost, 0);
}

/// An alarm already firing when the daemon starts is told at its first
/// look (the deployment's word on the clocks, so its owner's GET reply is
/// aged), with its document's severity. Its owner then closes cleanly
/// (alarms.v1 §2.9: a close leaves its alarms): the alarm is last-known,
/// unobservable — a `lost_sight`, never a resolve, never ok.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_alarm_firing_at_the_first_look_is_told_and_its_owner_gone_is_never_a_resolve() {
    let (_router, ep) = zk2::router().await;
    let owners = zk2::client_ns(&ep, Some(NS)).await;
    let tool = zk2::client_ns(&ep, Some(NS)).await;
    let (svc, alarms) = owner(&owners).await;
    zk2::tokens(&tool, &format!("zk2/{OWNER}/@zk/instance/*"), 1).await;
    alarms
        .raise(alarm_a(Severity::Critical, "link to r2 down on eth0"))
        .await
        .expect("A raised");

    let ops = Capture::default();
    let running = engine::start(
        zk2::daemon_bus(&ep, NS).await,
        vec![alerts_rule("fleet-alerts", &["ops"])],
        ops.clone(),
        Options {
            clocks_synced: true,
            ..Options::default()
        },
    )
    .await;
    let got = about_a(&ops, "fleet-alerts", 1).await;
    let n = &got[0];
    assert_eq!(
        (n.kind, n.state, n.prior),
        (NoticeKind::Alert, CondState::Firing, None)
    );
    assert_eq!(n.severity, "critical", "the document's, read by the GET");
    let t = n.alert.as_ref().expect("the alarm's change");
    assert_eq!(
        t.change,
        AlarmChange::Raised,
        "a late reader's raise (§2.6)"
    );
    assert_eq!(
        serde_json::to_value(t.via).unwrap(),
        serde_json::json!("get")
    );
    assert!(
        n.message.contains("firing since "),
        "the document's timestamp: {}",
        n.message
    );

    // The owner closes cleanly: its alarm is last-known, never resolved.
    drop(alarms);
    svc.close().await.expect("the owner closes");
    let got = about_a(&ops, "fleet-alerts", 2).await;
    let n = &got[1];
    assert_eq!(n.kind, NoticeKind::LostSight, "firing → unobservable");
    assert_eq!(
        (n.state, n.prior),
        (CondState::Unobservable, Some(CondState::Firing))
    );
    assert!(
        n.evidence.contains("absent") && n.evidence.contains("last-known"),
        "{}",
        n.evidence
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    let all = of_a(&ops.lock().unwrap(), "fleet-alerts");
    assert!(
        all.iter().all(|n| n.state != CondState::Ok),
        "an absent owner's alarm is never resolved, never ok: {all:?}"
    );
    running.stop().await;
}

/// A restart with a state file is no news: the alarm the ledger remembers
/// announced firing, still firing at the second run's first look, is a
/// duplicate; its resolve, heard by the second run, is one `resolved`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restart_with_a_state_file_is_no_news_and_the_resolve_after_it_is_one() {
    let (_router, ep) = zk2::router().await;
    let owners = zk2::client_ns(&ep, Some(NS)).await;
    let tool = zk2::client_ns(&ep, Some(NS)).await;
    let (_svc, alarms) = owner(&owners).await;
    zk2::tokens(&tool, &format!("zk2/{OWNER}/@zk/instance/*"), 1).await;
    alarms
        .raise(alarm_a(Severity::Warning, "link to r2 down on eth0"))
        .await
        .expect("A raised");
    let dir = tempfile::tempdir().expect("a state dir");
    let state = dir.path().join("state.json");
    let opts = || Options {
        clocks_synced: true,
        state_file: Some(state.clone()),
        ..Options::default()
    };

    let first = Capture::default();
    let running = engine::start(
        zk2::daemon_bus(&ep, NS).await,
        vec![alerts_rule("fleet-alerts", &["ops"])],
        first.clone(),
        opts(),
    )
    .await;
    about_a(&first, "fleet-alerts", 1).await;
    running.stop().await;
    let saved = std::fs::read_to_string(&state).expect("the ledger was saved");
    assert!(
        saved.contains(&format!("fleet-alerts:{}", a_key())),
        "{saved}"
    );
    assert!(saved.contains("\"version\": 2") || saved.contains("\"version\":2"));

    let second = Capture::default();
    let running = engine::start(
        zk2::daemon_bus(&ep, NS).await,
        vec![alerts_rule("fleet-alerts", &["ops"])],
        second.clone(),
        opts(),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        of_a(&second.lock().unwrap(), "fleet-alerts").is_empty(),
        "still firing after the restart: a duplicate"
    );
    alarms.resolve(A_KEY).await.expect("A resolved");
    let got = about_a(&second, "fleet-alerts", 1).await;
    assert_eq!(
        (got[0].kind, got[0].state, got[0].prior),
        (NoticeKind::Resolved, CondState::Ok, Some(CondState::Firing))
    );
    running.stop().await;
}

/// `alert-firing` runs as zenctl's watchdog runs it (#800, AC), now in
/// zenwatch: one state for the rule, §5's second question at its floor —
/// firing while A fires at or above it; ok once A's resolve is heard,
/// while the owner's last GET, which answered A, stands (`answered`); then
/// unobservable, once its next GET draws nothing: an empty answer is no
/// verdict until a window longer than 900 s (§2.15, core O5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn alert_firing_runs_as_the_watchdog_runs_it() {
    let (_router, ep) = zk2::router().await;
    let owners = zk2::client_ns(&ep, Some(NS)).await;
    let tool = zk2::client_ns(&ep, Some(NS)).await;
    let (_svc, alarms) = owner(&owners).await;
    zk2::tokens(&tool, &format!("zk2/{OWNER}/@zk/instance/*"), 1).await;
    alarms
        .raise(alarm_a(Severity::Warning, "link to r2 down on eth0"))
        .await
        .expect("A raised");

    let ops = Capture::default();
    let running = engine::start(
        zk2::daemon_bus(&ep, NS).await,
        vec![engine::rule(
            "lab-warning",
            "alert-firing zk2/lab/** warning",
            "error",
        )],
        ops.clone(),
        Options {
            clocks_synced: true,
            ..Options::default()
        },
    )
    .await;
    let firing = |s: &[Outgoing]| s.iter().any(|o| o.notification.state == CondState::Firing);
    let got = engine::until(&ops, "alert-firing fires", firing).await;
    let n = got
        .iter()
        .map(|o| &o.notification)
        .find(|n| n.state == CondState::Firing)
        .expect("firing");
    assert_eq!(n.rule_kind, "alert-firing");
    assert_eq!(n.kind, NoticeKind::Transition);
    assert_eq!(n.severity, "error", "the rule's: one state for the rule");
    assert!(n.evidence.contains("alarms.v1 §2.15"), "{}", n.evidence);
    assert!(
        n.alert.is_none(),
        "a rule's state carries no alarm's change"
    );

    alarms.resolve(A_KEY).await.expect("A resolved");
    let got = engine::until(&ops, "alert-firing goes unobservable", |s| {
        s.last()
            .is_some_and(|o| o.notification.state == CondState::Unobservable)
    })
    .await;
    let seen: Vec<(NoticeKind, CondState)> = got
        .iter()
        .map(|o| (o.notification.kind, o.notification.state))
        .collect();
    assert_eq!(seen[0], (NoticeKind::Transition, CondState::Firing));
    assert!(
        matches!(
            &seen[1..],
            [
                (NoticeKind::Resolved, CondState::Ok),
                (NoticeKind::Unobservable, CondState::Unobservable)
            ] | [(NoticeKind::LostSight, CondState::Unobservable)]
        ),
        "firing, then ok while the GET that answered A stands, then no verdict: {seen:?}"
    );
    running.stop().await;
}

/// A lost reading (core §2.7, 0.27): the router restarts, so the daemon's
/// transports close — a stall, a loss at the instant its reader sees it.
/// The rule's observer says it once, `unobservable`; A keeps its last word
/// through it (never resolved, never a second firing), and the GET after
/// the clients reconnect heals the loss: `observable_again`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lost_reading_is_said_once_by_the_observer_and_the_get_after_it_heals_it() {
    let (router, ep) = zk2::router().await;
    let owners = zk2::client_ns(&ep, Some(NS)).await;
    let tool = zk2::client_ns(&ep, Some(NS)).await;
    let (_svc, alarms) = owner(&owners).await;
    zk2::tokens(&tool, &format!("zk2/{OWNER}/@zk/instance/*"), 1).await;
    alarms
        .raise(alarm_a(Severity::Warning, "link to r2 down on eth0"))
        .await
        .expect("A raised");
    let ops = Capture::default();
    let running = engine::start(
        zk2::daemon_bus(&ep, NS).await,
        vec![alerts_rule("fleet-alerts", &["ops"])],
        ops.clone(),
        Options {
            clocks_synced: true,
            ..Options::default()
        },
    )
    .await;
    about_a(&ops, "fleet-alerts", 1).await;

    // The router goes, and comes back on its endpoint.
    router.close().await.expect("the router closes");
    let observer = "fleet-alerts:observer";
    let lost = engine::until(&ops, "the observer says the loss", |s| {
        s.iter().any(|o| {
            o.notification.id == observer && o.notification.state == CondState::Unobservable
        })
    })
    .await;
    let n = lost
        .iter()
        .map(|o| &o.notification)
        .find(|n| n.id == observer)
        .expect("the observer");
    assert_eq!(n.kind, NoticeKind::Unobservable);
    assert!(n.evidence.contains("§2.14 step 4"), "{}", n.evidence);
    let mut c = zenoh::Config::default();
    c.insert_json5("scouting/multicast/enabled", "false")
        .expect("config");
    c.insert_json5("scouting/gossip/enabled", "false")
        .expect("config");
    c.insert_json5("mode", "\"router\"").expect("config");
    c.insert_json5("listen/endpoints", &format!("[\"{ep}\"]"))
        .expect("config");
    let _router = zenoh::open(c).await.expect("the router again");
    engine::until(&ops, "the observer sees again", |s| {
        s.iter().any(|o| {
            o.notification.id == observer && o.notification.kind == NoticeKind::ObservableAgain
        })
    })
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let all = of_a(&ops.lock().unwrap(), "fleet-alerts");
    assert_eq!(
        all.len(),
        1,
        "A kept its word through the loss, and the healing GET found it firing: {all:?}"
    );
    running.stop().await;
}

/// A daemon with an `alerts` rule is the zk2 service it always is: its
/// `firing/{rule_id}` names the alarm that fires, under its identity and
/// its document's severity, and its `health.v1` carries an `alarms` check,
/// OK while its reader reads whole.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_daemon_with_alerts_publishes_the_alarm_and_its_alarms_check() {
    use util::engine::TestPublish;
    use zenwatch::zk2::zenwatch_v1;
    const ADDRESS: &str = "ops-1/zenwatch";
    let (_router, ep) = zk2::router().await;
    let owners = zk2::client_ns(&ep, Some(NS)).await;
    let tool = zk2::client_ns(&ep, Some(NS)).await;
    let (_svc, alarms) = owner(&owners).await;
    zk2::tokens(&tool, &format!("zk2/{OWNER}/@zk/instance/*"), 1).await;
    alarms
        .raise(alarm_a(Severity::Warning, "link to r2 down on eth0"))
        .await
        .expect("A raised");
    let ops = Capture::default();
    let running = engine::start(
        zk2::daemon_bus(&ep, NS).await,
        vec![alerts_rule("fleet-alerts", &["ops"])],
        ops.clone(),
        Options {
            clocks_synced: true,
            publish: Some(TestPublish::at(ADDRESS)),
            ..Options::default()
        },
    )
    .await;
    about_a(&ops, "fleet-alerts", 1).await;
    zk2::tokens(&tool, "zk2/ops-1/zenwatch/@zk/instance/*", 1).await;

    let zw = zenwatch_v1::Consumer::from_inner(
        zenkey::consumer::Consumer::for_tool(
            &tool,
            zenwatch_v1::contract(),
            &[ADDRESS],
            &BTreeMap::new(),
        )
        .expect("a tool's consumer"),
    )
    .expect("a zenwatch.v1 consumer");
    let id = zenwatch::rules::rule_id("fleet-alerts");
    zk2::eventually("the alarm's firing document", || async {
        match zw.get_firing_member(&id, zk2::T).await {
            Ok(zenkey::typed::StateGet::Answered(v)) => v.iter().any(|c| {
                matches!(c, zenkey::typed::Current::Value { value: Ok(doc), .. }
                    if doc.id == format!("fleet-alerts:{}", a_key())
                        && doc.state == "firing"
                        && doc.severity == "warning")
            }),
            _ => false,
        }
    })
    .await;

    let report = zenkey_fleet::run_health(
        &tool,
        NS,
        zenkey_fleet::HealthTarget::One(ADDRESS.parse().expect("an address")),
        zenkey_fleet::HealthSpec {
            timeout: zk2::T,
            window: None,
            grace: Duration::from_millis(100),
            clocks_synced: true,
            face: None,
        },
        || {},
    )
    .await;
    let row = report.services.first().expect("the daemon's row");
    let alarms_check = row
        .checks
        .iter()
        .find(|c| c.check == "alarms")
        .unwrap_or_else(|| panic!("an alarms check: {row:?}"));
    assert_eq!(
        alarms_check.level,
        zenkey_fleet::report::LevelRead::Token("ok"),
        "{alarms_check:?}"
    );
    assert!(
        alarms_check.detail.contains("a complete read"),
        "{alarms_check:?}"
    );
    running.stop().await;
}

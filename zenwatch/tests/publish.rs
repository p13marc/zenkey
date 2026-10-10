//! Self-publication against a real bus (#389; zk2 since #614, FL3): the
//! daemon is a zk2 service, and any explorer reads it through the fleet
//! without SSH — its descriptor lists `zenwatch.v1` and `health.v1`, its
//! health is judged by `health.v1`'s reader, a firing rule is one
//! `firing/{rule_id}` document and its resolve the delete, a failing sink
//! raises its status to `DEGRADED`, and after a stop its tokens are gone.
//! It passes its own conformance; the 257th firing rule is refused and
//! counted; its own clean stop pages nobody, while a second zenwatch
//! watching it sees it go; and a host with no id refuses to start it
//! unless `publish.ephemeral` says otherwise.
//!
//! One router, the daemon's two sessions and the explorers as its
//! clients; ports are ephemeral.

mod util;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use util::engine::{self, Capture, Options, TestPublish};
use util::zk2;
use zenkey::model::grammar::Addr;
use zenkey_fleet::report::{CaseId, HealthLevel, HealthVerdict};
use zenkey_fleet::{HealthSpec, HealthTarget, PresenceScope};
use zenoh::sample::SampleKind;
use zenwatch::config::{PublishSettings, RuleConfig, SinkConfig};
use zenwatch::publish::{FIRING_CARDINALITY, FiringRule, SelfService, ZenwatchCounters};
use zenwatch::sinks::Sink;
use zenwatch::zk2::zenwatch_v1;

const TOKEN: &str = "demo/held/p1/alive";
const NS: &str = "site";
const ADDRESS: &str = "ops-1/zenwatch";

/// What the explorer's subscriber saw: key, kind, body.
type Seen = Arc<Mutex<Vec<(String, SampleKind, Vec<u8>)>>>;

/// Until `pred` holds over the samples seen so far (or SETTLE).
async fn until(seen: &Seen, what: &str, pred: impl Fn(&[(String, SampleKind, Vec<u8>)]) -> bool) {
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    loop {
        if pred(&seen.lock().unwrap()) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "waited {:?} for {what}; saw {:?}",
            util::SETTLE,
            seen.lock()
                .unwrap()
                .iter()
                .map(|(k, kind, _)| format!("{kind:?} {k}"))
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn subscribe(
    session: &zenoh::Session,
    selector: &str,
) -> (Seen, zenoh::pubsub::Subscriber<()>) {
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let sub = {
        let seen = Arc::clone(&seen);
        session
            .declare_subscriber(selector.to_string())
            .callback(move |s| {
                seen.lock().unwrap().push((
                    s.key_expr().as_str().to_string(),
                    s.kind(),
                    s.payload().to_bytes().to_vec(),
                ));
            })
            .await
            .unwrap()
    };
    (seen, sub)
}

/// A sink whose every delivery fails: a program that is not there.
fn failing(name: &str) -> Sink {
    Sink::build(
        name,
        &SinkConfig::Exec {
            program: "/nonexistent/zenwatch-test-pager".into(),
            args: vec![],
            timeout_s: Some(5.0),
        },
        &zenwatch::sinks::http::client(),
    )
    .expect("an exec sink")
}

fn addr(s: &str) -> Addr {
    s.parse().expect("an address")
}

/// The service's health, as `health.v1`'s reader judges it
/// (`zenkey_model::health::judge`, through the fleet's reading). The
/// tool's clock is the owner's here (one process), which is the
/// operator's word `clocks_synced` gives.
async fn health_of(tool: &zenoh::Session, at: &Addr) -> zenkey_fleet::report::HealthRow {
    let report = zenkey_fleet::run_health(
        tool,
        NS,
        HealthTarget::One(at.clone()),
        HealthSpec {
            timeout: zk2::T,
            window: None,
            grace: Duration::from_millis(100),
            clocks_synced: true,
            face: None,
        },
    )
    .await;
    assert_eq!(report.services.len(), 1, "{report:?}");
    report.services.into_iter().next().unwrap()
}

/// The generated consumer of `zenwatch.v1`, bound to the daemon as a tool
/// binds (R1).
fn consumer(tool: &zenoh::Session, at: &str) -> zenwatch_v1::Consumer {
    zenwatch_v1::Consumer::from_inner(
        zenkey::consumer::Consumer::for_tool(
            tool,
            zenwatch_v1::contract(),
            &[at],
            &BTreeMap::new(),
        )
        .expect("a tool's consumer"),
    )
    .expect("a zenwatch.v1 consumer")
}

/// The values a GET answered.
fn values<T: Clone>(got: zenkey::typed::StateGet<T>) -> Vec<T> {
    match got {
        zenkey::typed::StateGet::Answered(v) => v
            .into_iter()
            .filter_map(|c| match c {
                zenkey::typed::Current::Value { value, .. } => value.ok(),
                zenkey::typed::Current::Deleted { .. } => None,
            })
            .collect(),
        zenkey::typed::StateGet::Silent => Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_daemon_is_a_zk2_service_the_fleet_reads() {
    let (_router, ep) = zk2::router().await;
    let raw = zk2::client(&ep).await;
    let tool = zk2::client_ns(&ep, Some(NS)).await;

    // The explorer watches everything zenwatch.v1 publishes, in the
    // deployment's namespace, before the daemon exists (O4).
    let (seen, _sub) = subscribe(&tool, "zk2/ops-1/zenwatch/zenwatch.v1/state/**").await;
    // A foreign token, held before the daemon joins: its going is what
    // fires, to a sink that works and one that does not.
    let token = raw.liveliness().declare_token(TOKEN).await.unwrap();
    zk2::tokens(&raw, TOKEN, 1).await;

    let ops = Capture::default();
    let running = engine::start(
        zk2::daemon_bus(&ep, NS).await,
        vec![RuleConfig {
            sinks: vec!["ops".into(), "pager".into()],
            ..engine::rule("held-gone", "liveliness-gone demo/held/*/alive", "error")
        }],
        ops.clone(),
        Options {
            publish: Some(TestPublish::at(ADDRESS)),
            extra_sinks: vec![failing("pager")],
            ..Options::default()
        },
    )
    .await;
    let at = addr(ADDRESS);

    // 1. Through the fleet: present, and its descriptor lists both
    // interfaces at the revisions it serves; the doctor, not configured,
    // is unavailable for that cause; freshness.v1 is a profile it follows.
    let observed = zenkey_fleet::observe_presence(&tool, &PresenceScope::service(&at), zk2::T)
        .await
        .unwrap();
    assert!(observed.complete);
    let descriptors = observed.descriptors.expect("descriptors asked");
    assert_eq!(descriptors.len(), 1, "one instance");
    let d = descriptors
        .values()
        .next()
        .unwrap()
        .descriptor()
        .expect("served")
        .clone();
    assert_eq!(d.service, ADDRESS);
    let iface = |name: &str| d.interfaces.iter().find(|e| e.iface == name);
    let zw = iface("zenwatch.v1").expect("zenwatch.v1 listed");
    assert_eq!(zw.contract, zenwatch_v1::FINGERPRINT);
    assert!(zw.token);
    assert_eq!(zw.unavailable.len(), 1, "{:?}", zw.unavailable);
    assert_eq!(zw.unavailable[0].resource, zenwatch_v1::resource::DOCTOR);
    assert_eq!(
        zw.unavailable[0].cause,
        zenkey::model::descriptor::Cause::Config
    );
    assert!(
        zw.cardinality.is_empty(),
        "the contract's bound, not lowered"
    );
    let health = iface("health.v1").expect("health.v1 listed");
    assert_eq!(
        health.contract,
        format!("sha256:{}", zenkey::health::FINGERPRINT)
    );
    assert!(
        d.profiles.iter().any(|p| p == "freshness.v1"),
        "{:?}",
        d.profiles
    );
    assert!(
        !d.profiles.iter().any(|p| p == "hostid.v1"),
        "a literal address is not minted"
    );

    // 2. Its health, judged: OK, its status declared and no check worse.
    let row = health_of(&tool, &at).await;
    assert_eq!(row.verdict, HealthVerdict::Healthy, "{row:?}");
    assert_eq!(row.level, Some(HealthLevel::Ok));
    let check_names: Vec<&str> = row.checks.iter().map(|c| c.check.as_str()).collect();
    assert_eq!(check_names, ["firing", "sinks", "tokens"], "{row:?}");

    // …and its counters, through the generated consumer (S4).
    // …and its counters, put before its tokens (core §8.2, "State
    // values"), through the generated consumer (S4).
    let zw = consumer(&tool, ADDRESS);
    let counters: Vec<ZenwatchCounters> = values(zw.get_counters(zk2::T).await.unwrap());
    assert_eq!(counters.len(), 1);
    assert_eq!((counters[0].rules, counters[0].sinks), (1, 2));
    assert_eq!(counters[0].firing, 0);

    // 3. The token gone: one firing/{rule_id} document — the rule's id is
    // its name slugged — and both sinks tried.
    token.undeclare().await.unwrap();
    let firing_key = format!(
        "{NS}/zk2/ops-1/zenwatch/zenwatch.v1/state/firing/{}",
        zenwatch::rules::rule_id("held-gone")
    );
    let firing_key_rel = firing_key.trim_start_matches(&format!("{NS}/")).to_string();
    until(&seen, "the firing document", |s| {
        s.iter()
            .any(|(k, kind, _)| *k == firing_key_rel && *kind == SampleKind::Put)
    })
    .await;
    let doc: FiringRule = {
        let s = seen.lock().unwrap();
        let (_, _, body) = s.iter().find(|(k, _, _)| *k == firing_key_rel).unwrap();
        serde_json::from_slice(body).unwrap()
    };
    assert_eq!(doc.rule, "held-gone");
    assert_eq!(doc.id, format!("held-gone:{TOKEN}"));
    assert_eq!(doc.state, "firing");
    assert_eq!(doc.severity, "error");
    assert_eq!(doc.count, 1);
    let got = values(
        zw.get_firing_member(&zenwatch::rules::rule_id("held-gone"), zk2::T)
            .await
            .unwrap(),
    );
    assert_eq!(
        got,
        std::slice::from_ref(&doc),
        "the owner answers what it put (S2)"
    );
    engine::until(&ops, "the working sink's notification", |s| s.len() == 1).await;

    // 4. The pager failed: the `sinks` check is DEGRADED, and the runtime
    // raised the status to it (health.v1 §2.2) — unhealthy, never
    // inconsistent.
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    let row = loop {
        let row = health_of(&tool, &at).await;
        if row.verdict == HealthVerdict::Unhealthy {
            break row;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the failing sink never showed: {row:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(row.level, Some(HealthLevel::Degraded), "{row:?}");
    assert_eq!(row.reason, "degraded", "{row:?}");
    let sinks = row.checks.iter().find(|c| c.check == "sinks").unwrap();
    assert!(
        sinks.detail.contains("1 of 2 sink(s)") && sinks.detail.contains("pager"),
        "{sinks:?}"
    );
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    loop {
        let c: Vec<ZenwatchCounters> = values(zw.get_counters(zk2::T).await.unwrap());
        if c.first()
            .is_some_and(|c| c.deliveries_failed == 1 && c.delivered == 1)
        {
            assert_eq!(c[0].firing, 1);
            assert_eq!(c[0].discipline.sent, 1);
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "counters: {c:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // 5. The token back: the document deleted.
    let token = raw.liveliness().declare_token(TOKEN).await.unwrap();
    until(&seen, "the firing delete", |s| {
        s.iter()
            .any(|(k, kind, _)| *k == firing_key_rel && *kind == SampleKind::Delete)
    })
    .await;

    // 6. Stop: the summary names the address, and its tokens are gone.
    let summary = running.stop().await;
    assert_eq!(summary.self_address.as_deref(), Some(ADDRESS));
    assert_eq!(summary.firing_refused, 0);
    zk2::tokens(&tool, "zk2/ops-1/zenwatch/@zk/**", 0).await;
    token.undeclare().await.unwrap();
}

/// zenwatch passes its own conformance (zk2's `check conform`, #703): each
/// interface it implements, read through a session in the namespace as its
/// consumers read it, with a rule firing so `firing/{rule_id}` has a member.
/// No case finds anything; the cases a short window cannot establish are
/// unobservable and named here.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn zenwatch_passes_its_own_conformance() {
    let (_router, ep) = zk2::router().await;
    let raw = zk2::client(&ep).await;
    let token = raw.liveliness().declare_token(TOKEN).await.unwrap();
    zk2::tokens(&raw, TOKEN, 1).await;
    let ops = Capture::default();
    let running = engine::start(
        zk2::daemon_bus(&ep, NS).await,
        vec![engine::rule(
            "held-gone",
            "liveliness-gone demo/held/*/alive",
            "error",
        )],
        ops.clone(),
        Options {
            publish: Some(TestPublish::at(ADDRESS)),
            ..Options::default()
        },
    )
    .await;
    token.undeclare().await.unwrap();
    engine::until(&ops, "the firing", |s| !s.is_empty()).await;
    // A tick for the document to be put.
    tokio::time::sleep(Duration::from_millis(800)).await;

    let bus = zk2::daemon_bus(&ep, NS).await;
    for iface in ["zenwatch.v1", "health.v1"] {
        let report = zenkey_fleet::run_conform(
            &bus,
            &zenkey_fleet::ContractSet::new(),
            addr(ADDRESS),
            iface.parse().unwrap(),
            None,
            zenkey_fleet::ConformSpec {
                timeout: zk2::T,
                window: Duration::from_secs(2),
                call_all: false,
                seed: 0,
                trust_admin: false,
                calls_granted: true,
                clocks_synced: true,
                lease: zenkey::model::budget::DEFAULT_LEASE,
            },
        )
        .await;
        assert!(report.unobservable.is_none(), "{iface}: {report:#?}");
        let failures: Vec<_> = report.failures().collect();
        assert!(failures.is_empty(), "{iface}: {failures:#?}");
        let unobservable: Vec<(CaseId, String)> = report
            .cases
            .iter()
            .filter(|c| c.verdict.is_unobservable())
            .map(|c| (c.case, c.subject.clone()))
            .collect();
        for c in report.cases.iter().filter(|c| c.verdict.is_unobservable()) {
            eprintln!("{iface}: {:?} {}: {:?}", c.case, c.subject, c.verdict);
        }
        // Every unobservable case is one this reading cannot establish and
        // says why: a state's stamp proved against routers whose admin
        // answers it was not told to trust (S1, §4.2), QoS on a window that
        // heard no put (state is put on change, and re-put every ttl/2:
        // longer than the window), and health.v1's faults stream, silent
        // as it should be when nothing happened (health.v1 §2.3).
        for (case, subject) in &unobservable {
            let explained = matches!(case, CaseId::StateStamp | CaseId::Qos)
                || (subject == "stream/faults"
                    && matches!(case, CaseId::ResourceServed | CaseId::PayloadType));
            assert!(explained, "{iface}: {case:?} {subject} unobservable");
        }
        assert!(
            report
                .cases
                .iter()
                .any(|c| c.case == CaseId::ContractServed && c.verdict.conclusive() == Some(false)),
            "{iface}: its bundle served: {report:#?}"
        );
    }
    running.stop().await;
}

/// The 257th firing rule (core §2.7): the runtime refuses its document at
/// the contract's bound with `Error::Budget`, nothing is published, and the
/// refusal is counted once — never silent; a member freed lets it in.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_257th_firing_rule_is_refused_and_counted() {
    let (_router, ep) = zk2::router().await;
    let owner = zk2::client_ns(&ep, Some(NS)).await;
    let tool = zk2::client_ns(&ep, Some(NS)).await;
    let p = TestPublish::at(ADDRESS);
    let sinks = BTreeMap::from([("ops".to_string(), None)]);
    let checks = zenwatch::publish::checks(&zenwatch::publish::HealthInputs {
        sinks: &sinks,
        presence: None,
        tokens: None,
        doctor: None,
        state_file: None,
        firing_refused: 0,
    });
    let mut svc = SelfService::start(
        &owner,
        p.config.clone(),
        &p.hostid,
        &zenwatch::publish::Plan {
            rules: 257,
            sinks: 1,
            doctor: false,
        },
        &checks,
        &ZenwatchCounters {
            started_at: "2026-10-10T00:00:00Z".into(),
            rules: 257,
            sinks: 1,
            firing: 0,
            unobservable: 0,
            delivered: 0,
            deliveries_failed: 0,
            dropped: 0,
            forgotten: 0,
            state_entries: 0,
            firing_refused: 0,
            doctor_runs: 0,
            discipline: Default::default(),
        },
    )
    .await
    .expect("the service");
    let doc = |i: u64| FiringRule {
        rule: format!("rule-{i:03}"),
        id: format!("rule-{i:03}:x"),
        state: "firing".into(),
        severity: "error".into(),
        since: "2026-10-10T00:00:00Z".into(),
        labels: BTreeMap::new(),
        count: 1,
    };
    let mut docs: BTreeMap<String, FiringRule> = (0..=FIRING_CARDINALITY)
        .map(|i| (format!("rule-{i:03}"), doc(i)))
        .collect();
    assert_eq!(docs.len(), 257);
    svc.publish_firing(&docs)
        .await
        .expect("refusals are not errors");
    assert_eq!(svc.firing_refused(), 1, "the 257th, counted");
    assert_eq!(svc.firing_refused_now(), 1);
    let zw = consumer(&tool, ADDRESS);
    let published = values(zw.get_firing(zk2::T).await.unwrap());
    assert_eq!(
        published.len() as u64,
        FIRING_CARDINALITY,
        "never past the bound"
    );
    assert!(
        !published.iter().any(|d| d.rule == "rule-256"),
        "the last in order is the one refused"
    );

    // The next tick tries again and counts nothing new: one refusal, one
    // count, however long it lasts.
    svc.publish_firing(&docs).await.unwrap();
    assert_eq!(svc.firing_refused(), 1);
    // The `firing` check says so while it lasts.
    let checks = zenwatch::publish::checks(&zenwatch::publish::HealthInputs {
        sinks: &sinks,
        presence: None,
        tokens: None,
        doctor: None,
        state_file: None,
        firing_refused: svc.firing_refused_now(),
    });
    let firing = checks.iter().find(|c| c.name == "firing").unwrap();
    assert_eq!(firing.level, Some(zenkey::health::Level::Degraded));
    assert!(firing.detail.contains("1 rule(s)"), "{}", firing.detail);

    // A rule goes quiet: its member frees, and the refused one is put.
    docs.remove("rule-000");
    svc.publish_firing(&docs).await.unwrap();
    assert_eq!(svc.firing_refused_now(), 0);
    assert_eq!(svc.firing_refused(), 1);
    let published = values(zw.get_firing(zk2::T).await.unwrap());
    assert_eq!(published.len() as u64, FIRING_CARDINALITY);
    assert!(published.iter().any(|d| d.rule == "rule-256"));
    svc.shutdown().await.unwrap();
}

/// Its own clean stop is not news (#614, FL3): a daemon whose rule covers
/// its own address (`instance-gone */*`, not excluded by special case)
/// stops its observers and drains its sinks before its service withdraws
/// its tokens, so it pages nobody about itself and its state file does not
/// record it gone — while a second zenwatch watching it, as two notifiers
/// watching each other do, sees it go.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn its_own_clean_stop_pages_nobody_but_a_watcher_sees_it_go() {
    let (_router, ep) = zk2::router().await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("zenwatch-state.json");

    // The watcher: no service of its own, the same kind of rule.
    let watcher_ops = Capture::default();
    let watcher = engine::start(
        zk2::daemon_bus(&ep, NS).await,
        vec![engine::rule(
            "watch-gone",
            "instance-gone */zenwatch",
            "error",
        )],
        watcher_ops.clone(),
        Options::default(),
    )
    .await;

    let ops = Capture::default();
    let running = engine::start(
        zk2::daemon_bus(&ep, NS).await,
        vec![engine::rule("anything-gone", "instance-gone */*", "error")],
        ops.clone(),
        Options {
            publish: Some(TestPublish::at(ADDRESS)),
            state_file: Some(state.clone()),
            ..Options::default()
        },
    )
    .await;
    let tool = zk2::client_ns(&ep, Some(NS)).await;
    zk2::tokens(&tool, "zk2/ops-1/zenwatch/@zk/instance/*", 1).await;
    // A few ticks with itself up: its first sight is remembered, not news.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let summary = running.stop().await;
    assert_eq!(summary.self_address.as_deref(), Some(ADDRESS));

    // The watcher sees it go…
    let got = engine::until(&watcher_ops, "the watcher's notification", |s| {
        s.iter().any(|o| o.notification.id.contains(ADDRESS))
    })
    .await;
    let gone = got
        .iter()
        .find(|o| o.notification.id.contains(ADDRESS))
        .unwrap();
    assert_eq!(gone.notification.state, zenkey_fleet::CondState::Firing);

    // …and the daemon said nothing about itself, then or ever.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let own: Vec<String> = ops
        .lock()
        .unwrap()
        .iter()
        .map(|o| o.notification.id.clone())
        .filter(|id| id.contains(ADDRESS))
        .collect();
    assert!(own.is_empty(), "it paged about its own stop: {own:?}");
    let saved = std::fs::read_to_string(&state).unwrap_or_default();
    assert!(
        !saved.contains(ADDRESS),
        "its state file records it gone: {saved}"
    );
    watcher.stop().await;
}

/// The address (`hostid.v1` §2.6, #614 FL3): with no `publish.address`, a
/// host with no id source refuses to start the daemon — exit 2, nothing
/// opened, every way out named — and `publish.ephemeral: true` starts it on
/// an ephemeral minted system, which its descriptor states.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_host_without_an_id_refuses_to_start_unless_ephemeral() {
    let (_router, ep) = zk2::router().await;
    let e = engine::run_once_refused(
        zk2::daemon_bus(&ep, NS).await,
        vec![engine::rule("observer-drops", "dropped", "info")],
        Options {
            publish: Some(TestPublish::on_a_host_without_an_id(
                &PublishSettings::default(),
            )),
            ..Options::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(zenwatch::exit::code_for(&e), zenwatch::exit::REFUSED, "{e}");
    let text = e.to_string();
    for needle in [
        "hostid.v1 §2.6",
        "@hostid.v1/zenwatch",
        "publish: { ephemeral: true }",
        "publish: { address:",
        "publish: false",
    ] {
        assert!(text.contains(needle), "{needle:?} not in {text}");
    }
    let tool = zk2::client_ns(&ep, Some(NS)).await;
    zk2::tokens(&tool, "zk2/*/zenwatch/@zk/**", 0).await;

    let running = engine::start(
        zk2::daemon_bus(&ep, NS).await,
        vec![engine::rule("observer-drops", "dropped", "info")],
        Capture::default(),
        Options {
            publish: Some(TestPublish::on_a_host_without_an_id(&PublishSettings {
                address: None,
                ephemeral: true,
            })),
            ..Options::default()
        },
    )
    .await;
    zk2::tokens(&tool, "zk2/*/zenwatch/@zk/instance/*", 1).await;
    let summary = running.stop().await;
    let at = summary.self_address.expect("it ran a service");
    let (system, service) = at.split_once('/').unwrap();
    assert_eq!(service, "zenwatch");
    assert!(
        system.starts_with("h-") && system.len() == 14,
        "a minted system: {system}"
    );
}

//! The discipline against a real bus (#389): a restart with the same state
//! file does not re-page an alert already announced, and inhibition runs
//! end to end over catalog documents — a host entity down, the alert on
//! the guest it hosts delivered as a symptom naming the host's entity.
//! Ports are ephemeral (`util::peer_pair`).

mod util;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zenkey::qos::QosProfile;
use zenkey_fleet::{BringUp, CondState, Fleet, Publication, declare_publication};
use zenwatch::config::{DisciplineConfig, RuleConfig};
use zenwatch::engine::{Engine, RunSummary, run_on};
use zenwatch::render::RenderConfig;
use zenwatch::rules::Rule;
use zenwatch::sinks::{NoticeKind, Outgoing, Sink};

type Capture = Arc<Mutex<Vec<Outgoing>>>;

struct Running {
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<anyhow::Result<RunSummary>>,
}

impl Running {
    async fn stop(self) -> RunSummary {
        let _ = self.stop.send(());
        tokio::time::timeout(util::SETTLE, self.task)
            .await
            .expect("the engine stopped")
            .expect("no panic")
            .expect("a clean run")
    }
}

/// An engine on `session` with `rules`, one capturing sink, no group
/// window, a half-second tick, and the given state file.
async fn start(
    session: &zenoh::Session,
    rules: Vec<RuleConfig>,
    state_file: Option<PathBuf>,
    ops: Capture,
) -> Running {
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    let session = session.clone();
    let task = tokio::spawn(async move {
        let rules: Vec<Rule> = rules
            .iter()
            .map(|r| Rule::from_config(r).unwrap())
            .collect();
        let render = RenderConfig::default();
        let discipline = DisciplineConfig {
            group_window_s: 0.0,
            ..DisciplineConfig::default()
        };
        run_on(
            Engine {
                fleet: Fleet::new(&session, ""),
                slices: None,
                timeout: Duration::from_secs(2),
                tick: Duration::from_millis(500),
                rules: &rules,
                sinks: Arc::new(vec![Sink::capturing("ops", ops)]),
                render: &render,
                once: false,
                ready: Some(ready_tx),
                discipline: &discipline,
                state_file,
                state_max_entries: 4096,
                publish: false,
            },
            async move {
                let _ = stop_rx.await;
            },
        )
        .await
    });
    tokio::time::timeout(util::SETTLE, ready_rx)
        .await
        .expect("the engine came up")
        .expect("ready");
    Running {
        stop: stop_tx,
        task,
    }
}

async fn settled(capture: &Capture, n: usize, what: &str) -> Vec<Outgoing> {
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    loop {
        let snapshot = capture.lock().unwrap().clone();
        if snapshot.len() >= n {
            return snapshot;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "waited {:?} for {what} ({n} deliveries); have {:?}",
            util::SETTLE,
            snapshot
                .iter()
                .map(|o| o.notification.id.clone())
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn alert_publication(session: &zenoh::Session, key: &str) -> Publication {
    let p = declare_publication(session, key, QosProfile::Alert, Some("application/json"))
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    while !p.matching_status().await.unwrap() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the observer never matched {key}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    p
}

fn alerts_rule(for_s: Option<f64>) -> RuleConfig {
    RuleConfig {
        name: "fleet-alerts".into(),
        rule: "alerts v1/*/state/*/alert/*".into(),
        severity: None,
        labels: Default::default(),
        sinks: vec!["ops".into()],
        for_s,
    }
}

fn liveliness_rule() -> RuleConfig {
    RuleConfig {
        name: "hosts-gone".into(),
        rule: "liveliness-gone v1/*/state/*/alive".into(),
        severity: Some("error".into()),
        labels: Default::default(),
        sinks: vec!["ops".into()],
        for_s: None,
    }
}

/// Run with a firing alert and a state file, stop, start again with the
/// same file, re-put the same alert: nothing — and the resolve after it
/// still arrives, because the ledger remembered the firing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_with_the_same_state_file_does_not_re_page() {
    const KEY: &str = "v1/h-3fa9c2d41b7e/state/netlink/alert/a659f813308ad1da";
    let (a, b) = util::peer_pair().await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("zenwatch-state.json");
    let doc = br#"{"severity":"warning","rule":"link_down","message":"eth0 is down"}"#;

    let first = Arc::new(Mutex::new(Vec::new()));
    let run1 = start(
        &b,
        vec![alerts_rule(None)],
        Some(state.clone()),
        Arc::clone(&first),
    )
    .await;
    let publication = alert_publication(&a, KEY).await;
    publication.send(doc.to_vec(), None).await.unwrap();
    let got = settled(&first, 1, "the first firing").await;
    assert_eq!(got[0].notification.kind, NoticeKind::Alert);
    let summary = run1.stop().await;
    assert_eq!(summary.outgoing, 1);
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
    assert_eq!(saved["version"], 1);
    assert_eq!(saved["entries"].as_array().unwrap().len(), 1, "{saved}");
    assert_eq!(saved["entries"][0]["state"], "firing");

    // The second life: the re-put is a duplicate of what was announced
    // before the restart.
    let second = Arc::new(Mutex::new(Vec::new()));
    let run2 = start(
        &b,
        vec![alerts_rule(None)],
        Some(state.clone()),
        Arc::clone(&second),
    )
    .await;
    let publication = alert_publication(&a, KEY).await;
    publication.send(doc.to_vec(), None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        second.lock().unwrap().is_empty(),
        "a re-put of an alert announced before the restart is nothing: {:?}",
        second.lock().unwrap()
    );
    // …and its resolve is one resolved, which proves the memory rather than
    // a lost subscription.
    publication.retire().await.unwrap();
    let got = settled(&second, 1, "the resolve after the restart").await;
    assert_eq!(got[0].notification.kind, NoticeKind::Resolved);
    assert_eq!(got[0].notification.state, CondState::Ok);
    let summary = run2.stop().await;
    assert_eq!(summary.discipline.deduped, 1);
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
    assert_eq!(
        saved["entries"].as_array().unwrap().len(),
        0,
        "resolved: nothing to remember"
    );
}

/// Inhibition end to end: two host entities and `A hosts B` from a test
/// session standing in for the catalog, two alive tokens, an alert on B's
/// origin waiting out a one-second `for`, then A's token retired — A's
/// liveliness-gone is the root, and B's alert is delivered as a symptom
/// naming A's entity.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_alert_on_a_guest_of_a_down_host_is_a_symptom_of_the_host() {
    const A: &str = "h-aaaaaaaaaaaa";
    const B: &str = "h-bbbbbbbbbbbb";
    let (a, b) = util::peer_pair().await;

    let ops = Arc::new(Mutex::new(Vec::new()));
    let run = start(
        &b,
        vec![alerts_rule(Some(1.0)), liveliness_rule()],
        None,
        Arc::clone(&ops),
    )
    .await;

    // The catalog's documents, in the reference profile's spelling
    // (zenkey_fleet::report::catalog pins it).
    let mut catalog = Vec::new();
    for (key, body) in [
        (
            "v1/@catalog/state/entity/ent-a",
            format!(r#"{{"entity_id":"ent-a","host_id":"{A}","origins":["{A}"],"hostname":"pve01"}}"#),
        ),
        (
            "v1/@catalog/state/entity/ent-b",
            format!(r#"{{"entity_id":"ent-b","origins":["{B}"],"hostname":"db01"}}"#),
        ),
        (
            "v1/@catalog/state/edge/e-2879d4667f9d946d",
            r#"{"edge_id":"e-2879d4667f9d946d","kind":"hosts","from":{"entity":{"entity_id":"ent-a"}},"to":{"entity":{"entity_id":"ent-b"}},"observers":[{"sensor":"pve","origin":"h-aaaaaaaaaaaa"}],"last_updated":1700000000}"#.to_string(),
        ),
    ] {
        let p = declare_publication(&a, key, QosProfile::Refreshed, Some("application/json"))
            .await
            .unwrap();
        let deadline = tokio::time::Instant::now() + util::SETTLE;
        while !p.matching_status().await.unwrap() {
            assert!(tokio::time::Instant::now() < deadline, "the catalog watch never matched {key}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        p.send(body.into_bytes(), None).await.unwrap();
        catalog.push(p);
    }

    // Two producers up: the hypervisor on A, the guest's app on B.
    let host = BringUp::new(&a)
        .alive(&format!("v1/{A}/state/pve/alive"))
        .await
        .unwrap();
    let guest = BringUp::new(&a)
        .alive(&format!("v1/{B}/state/app/alive"))
        .await
        .unwrap();
    // Let the roster and the catalog land before anything fires.
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(ops.lock().unwrap().is_empty(), "tokens up are the baseline");

    // The guest's alert (error, so a lone symptom is still delivered), then
    // the host goes down while the alert waits out its `for`.
    let alert_key = format!("v1/{B}/state/app/alert/db-down");
    let alert = alert_publication(&a, &alert_key).await;
    alert
        .send(
            br#"{"severity":"error","rule":"db_unreachable","message":"db01 unreachable"}"#
                .to_vec(),
            None,
        )
        .await
        .unwrap();
    host.retire().await.unwrap();

    let got = settled(&ops, 2, "the root and the symptom").await;
    let by_id = |needle: &str| {
        got.iter().find(|o| {
            o.notification.id.contains(needle)
                || o.notification
                    .group
                    .as_ref()
                    .is_some_and(|g| g.iter().any(|m| m.contains(needle)))
        })
    };
    let root = by_id(&format!("hosts-gone:{A}/pve")).expect("A's liveliness-gone");
    let symptom = by_id("fleet-alerts:h-bbbbbbbbbbbb.app.db-down").expect("B's alert");
    match symptom.notification.kind {
        NoticeKind::Group => {
            // Both landed in one flush: the root's group, root first.
            assert!(std::ptr::eq(root, symptom));
            let members = symptom.notification.group.as_ref().unwrap();
            assert!(members[0].starts_with("hosts-gone:"), "{members:?}");
            assert!(
                symptom.notification.message.contains("(symptom of ent-a)"),
                "{}",
                symptom.notification.message
            );
        }
        NoticeKind::Alert => {
            assert_eq!(symptom.notification.inhibited_by.as_deref(), Some("ent-a"));
            assert_eq!(symptom.notification.severity, "error");
            assert_eq!(root.notification.kind, NoticeKind::Liveliness);
            assert_eq!(
                root.notification.inhibited_by, None,
                "the root explains itself"
            );
        }
        other => panic!("unexpected kind {other:?} for the symptom: {symptom:?}"),
    }

    let summary = run.stop().await;
    assert_eq!(
        summary.discipline.inhibited, 0,
        "an error symptom is delivered, not held"
    );
    guest.retire().await.unwrap();
    for p in catalog {
        p.retire().await.unwrap();
    }
}

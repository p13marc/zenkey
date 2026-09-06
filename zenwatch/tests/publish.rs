//! Self-publication against a real bus (#389): the daemon comes up as a
//! producer on peer B, and peer A — any explorer — sees it without SSH:
//! `introspect` answers the registry slice naming the three subjects,
//! `describe` a schema set covering their three types, the roster lists
//! `zenwatch`, the health document arrives on its cadence, a firing rule is
//! one `firing/{rule_id}` document and its resolve the tombstone — and
//! after shutdown the token is gone. Ports are ephemeral (`util::peer_pair`).

mod util;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use zenkey::qos::QosProfile;
use zenkey::schema::SchemaSet;
use zenkey_fleet::bus::query::{Answer, GetOpts};
use zenkey_fleet::{Fleet, declare_publication, fleet_get, roster};
use zenoh::sample::SampleKind;
use zenwatch::config::DisciplineConfig;
use zenwatch::engine::{Engine, run_on};
use zenwatch::render::RenderConfig;
use zenwatch::rules::Rule;
use zenwatch::sinks::Sink;

const ALERT: &str = "v1/h-3fa9c2d41b7e/state/netlink/alert/a659f813308ad1da";

/// What the explorer's subscriber saw: key, kind, body.
type Seen = Arc<Mutex<Vec<(String, SampleKind, Vec<u8>)>>>;

/// Every reply to `selector` from one fleet GET, as UTF-8 bodies.
async fn ask(fleet: &Fleet<'_>, selector: &str) -> Vec<String> {
    fleet_get(fleet, selector, &GetOpts::new(Duration::from_secs(3)))
        .await
        .unwrap()
        .into_iter()
        .map(|a| match a.answer {
            Answer::Value(bytes) => String::from_utf8(bytes.to_bytes().to_vec()).unwrap(),
            Answer::Error { name, message } => panic!("{name}: {message}"),
        })
        .collect()
}

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_daemon_is_a_producer_an_explorer_can_see() {
    let (a, b) = util::peer_pair().await;

    // The explorer's side watches everything zenwatch publishes, before
    // the daemon exists (O4).
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let _sub = {
        let seen = Arc::clone(&seen);
        a.declare_subscriber("v1/*/state/zenwatch/**")
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

    let ops = Arc::new(Mutex::new(Vec::new()));
    let sinks = Arc::new(vec![Sink::capturing("ops", Arc::clone(&ops))]);
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    let observer = b.clone();
    let engine = tokio::spawn(async move {
        let rules = vec![
            Rule::from_config(&zenwatch::config::RuleConfig {
                name: "fleet-alerts".into(),
                rule: "alerts v1/*/state/*/alert/*".into(),
                severity: None,
                labels: Default::default(),
                sinks: vec!["ops".into()],
                for_s: None,
            })
            .unwrap(),
        ];
        let render = RenderConfig::default();
        let discipline = DisciplineConfig {
            group_window_s: 0.0,
            ..DisciplineConfig::default()
        };
        run_on(
            Engine {
                fleet: Fleet::new(&observer, ""),
                slices: None,
                timeout: Duration::from_secs(2),
                tick: Duration::from_millis(500),
                rules: &rules,
                sinks,
                render: &render,
                once: false,
                ready: Some(ready_tx),
                discipline: &discipline,
                state_file: None,
                state_max_entries: 4096,
                publish: true,
                doctor: None,
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

    let fleet = Fleet::new(&a, "");

    // The roster lists the notifier under a host origin.
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    let origin = loop {
        let r = roster(&fleet, Duration::from_millis(500)).await.unwrap();
        if let Some((origin, _)) = r
            .iter()
            .find(|(_, producers)| producers.iter().any(|p| p == "zenwatch"))
        {
            break origin.clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "zenwatch never joined the roster: {r:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(zenkey::grammar::is_valid_host_origin(&origin), "{origin}");

    // introspect: the registry slice, verbatim, naming the three subjects.
    let slices = ask(&fleet, &format!("v1/{origin}/@rpc/zenwatch/introspect")).await;
    assert_eq!(slices.len(), 1, "one producer, one slice");
    let slice = zenkey::parse_slice(&slices[0]).unwrap();
    assert_eq!(slice.name, "zenwatch");
    let paths: Vec<&str> = slice.subjects.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(paths, vec!["health", "firing/{rule_id}", "doctor"]);
    assert_eq!(
        slice
            .procedures
            .iter()
            .map(|p| p.path.as_str())
            .collect::<Vec<_>>(),
        vec!["introspect", "describe"]
    );

    // describe: a schema set covering the three types (RFC 08 §6.1).
    let described = ask(&fleet, &format!("v1/{origin}/@rpc/zenwatch/describe")).await;
    assert_eq!(described.len(), 1);
    let set = SchemaSet::parse(&described[0]).unwrap();
    assert_eq!(set.app(), "zenwatch");
    for t in ["ZenwatchHealth", "FiringRule", "ZenwatchDoctor"] {
        assert!(set.get(t).is_some(), "{t} described");
    }
    set.verify_covers(&zenwatch::publish::TYPE_NAMES).unwrap();

    // health, on the first tick, at status ok, carrying the origin as
    // host_id (RFC 06 §6.2).
    let health_key = format!("v1/{origin}/state/zenwatch/health");
    until(&seen, "the health document", |s| {
        s.iter()
            .any(|(k, kind, _)| k == &health_key && *kind == SampleKind::Put)
    })
    .await;
    let health: serde_json::Value = {
        let s = seen.lock().unwrap();
        let (_, _, body) = s.iter().find(|(k, _, _)| k == &health_key).unwrap();
        serde_json::from_slice(body).unwrap()
    };
    assert_eq!(health["status"], "ok");
    assert_eq!(health["host_id"], origin);
    assert_eq!(health["rules"], 1);
    assert_eq!(health["firing"], 0);
    assert_eq!(health["state_entries"], 0);

    // A firing alert is one firing/{rule_id} document — the rule's id is
    // its name slugged (`zenwatch::rules::rule_id`) — and its resolve is
    // the tombstone.
    let publication = declare_publication(&a, ALERT, QosProfile::Alert, Some("application/json"))
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    while !publication.matching_status().await.unwrap() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the observer never matched"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    publication
        .send(
            br#"{"severity":"error","rule":"link_down","message":"eth0 is down"}"#.to_vec(),
            None,
        )
        .await
        .unwrap();
    let firing_key = format!(
        "v1/{origin}/state/zenwatch/firing/{}",
        zenwatch::rules::rule_id("fleet-alerts")
    );
    until(&seen, "the firing document", |s| {
        s.iter()
            .any(|(k, kind, _)| k == &firing_key && *kind == SampleKind::Put)
    })
    .await;
    let doc: serde_json::Value = {
        let s = seen.lock().unwrap();
        let (_, _, body) = s.iter().find(|(k, _, _)| k == &firing_key).unwrap();
        serde_json::from_slice(body).unwrap()
    };
    assert_eq!(doc["rule"], "fleet-alerts");
    assert_eq!(
        doc["id"],
        "fleet-alerts:h-3fa9c2d41b7e.netlink.a659f813308ad1da"
    );
    assert_eq!(doc["state"], "firing");
    assert_eq!(doc["severity"], "error");
    assert_eq!(doc["count"], 1);
    assert_eq!(
        ops.lock().unwrap().len(),
        1,
        "and the sink got its notification"
    );

    publication.retire().await.unwrap();
    until(&seen, "the firing tombstone", |s| {
        s.iter()
            .any(|(k, kind, _)| k == &firing_key && *kind == SampleKind::Delete)
    })
    .await;

    // Shutdown: the token goes, the health is tombstoned.
    let _ = stop_tx.send(());
    let summary = tokio::time::timeout(util::SETTLE, engine)
        .await
        .expect("the engine stopped")
        .expect("no panic")
        .expect("a clean run");
    assert_eq!(summary.self_origin.as_deref(), Some(origin.as_str()));
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    loop {
        let r = roster(&fleet, Duration::from_millis(500)).await.unwrap();
        if !r.values().any(|p| p.iter().any(|p| p == "zenwatch")) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the token outlived the daemon: {r:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    until(&seen, "the health tombstone", |s| {
        s.iter()
            .any(|(k, kind, _)| k == &health_key && *kind == SampleKind::Delete)
    })
    .await;
}

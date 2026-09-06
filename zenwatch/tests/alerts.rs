//! `alerts <SEL>` against a real bus (#388): a producer's alert document
//! put on peer A is one firing notification per sink on peer B, a re-put
//! of the same document is nothing, the tombstone is one resolved, and a
//! re-fire after it is one firing again — with the document's severity,
//! rule and labels lifted, and the payload rendered structurally because no
//! schema is served. Ports are ephemeral (`util::peer_pair`).

mod util;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use zenkey::qos::QosProfile;
use zenkey_fleet::{CondState, Fleet, RenderSource, declare_publication};
use zenwatch::engine::{Engine, run_on};
use zenwatch::render::RenderConfig;
use zenwatch::rules::Rule;
use zenwatch::sinks::{Outgoing, Sink};

const KEY: &str = "v1/h-3fa9c2d41b7e/state/netlink/alert/a659f813308ad1da";

/// Wait until every capture holds at least `n` deliveries (or SETTLE).
async fn settled(captures: &[Arc<Mutex<Vec<Outgoing>>>], n: usize) -> Vec<Vec<Outgoing>> {
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    loop {
        let snapshot: Vec<Vec<Outgoing>> =
            captures.iter().map(|c| c.lock().unwrap().clone()).collect();
        if snapshot.iter().all(|v| v.len() >= n) {
            return snapshot;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "waited {:?} for {n} deliveries per sink; have {:?}",
            util::SETTLE,
            snapshot.iter().map(Vec::len).collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_alert_put_is_one_firing_per_sink_a_re_put_is_nothing_and_a_delete_resolves() {
    let (a, b) = util::peer_pair().await;

    let ops = Arc::new(Mutex::new(Vec::new()));
    let mail = Arc::new(Mutex::new(Vec::new()));
    let sinks = Arc::new(vec![
        Sink::capturing("ops", Arc::clone(&ops)),
        Sink::capturing("mail", Arc::clone(&mail)),
    ]);
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    let engine = tokio::spawn(async move {
        let rules = vec![
            Rule::from_config(&zenwatch::config::RuleConfig {
                name: "fleet-alerts".into(),
                rule: "alerts v1/*/state/*/alert/*".into(),
                severity: None,
                labels: [("team".to_string(), "infra".to_string())].into(),
                sinks: vec!["ops".into(), "mail".into()],
            })
            .unwrap(),
        ];
        let render = RenderConfig::default();
        run_on(
            Engine {
                fleet: Fleet::new(&b, ""),
                slices: None,
                timeout: Duration::from_secs(2),
                tick: Duration::from_secs(1),
                rules: &rules,
                sinks,
                render: &render,
                once: false,
                ready: Some(ready_tx),
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

    // The producer's side: a declared publication on the alert key, riding
    // the alert profile, saying it carries JSON.
    let publication = declare_publication(&a, KEY, QosProfile::Alert, Some("application/json"))
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
    let doc = br#"{"severity":"warning","rule":"link_down","labels":{"port":"eth0","host":"h-3fa9c2d41b7e"},"message":"eth0 is down"}"#;

    // 1. put → exactly one firing per sink, the document's fields lifted.
    publication.send(doc.to_vec(), None).await.unwrap();
    let got = settled(&[Arc::clone(&ops), Arc::clone(&mail)], 1).await;
    for sink in &got {
        assert_eq!(sink.len(), 1, "exactly one per sink");
        let n = &sink[0].notification;
        assert_eq!(n.state, CondState::Firing);
        assert_eq!(
            n.prior, None,
            "first sight: the baseline is stated, not invented"
        );
        assert_eq!(n.kind, "alerts");
        assert_eq!(n.rule, "fleet-alerts");
        assert_eq!(
            n.severity, "warning",
            "the producer's severity, not the rule's"
        );
        assert_eq!(n.labels.get("port").map(String::as_str), Some("eth0"));
        assert_eq!(n.labels.get("team").map(String::as_str), Some("infra"));
        assert!(
            !n.labels.contains_key("host"),
            "host is the origin's to say"
        );
        assert!(
            n.evidence
                .contains("h-3fa9c2d41b7e.netlink.a659f813308ad1da"),
            "{}",
            n.evidence
        );
        assert!(n.evidence.contains("rule=link_down"), "{}", n.evidence);
        assert!(n.evidence.contains("eth0 is down"), "{}", n.evidence);
        assert_eq!(n.rendering, RenderSource::Structural, "no served schema");
        assert!(n.message.contains("no served schema for"), "{}", n.message);
        assert!(
            n.message
                .contains("key: v1/h-3fa9c2d41b7e/state/netlink/alert/a659f813308ad1da")
        );
        assert_eq!(n.title, "warning fleet-alerts firing");
        assert_eq!(sink[0].sinks, vec!["ops".to_string(), "mail".to_string()]);
    }

    // 2. an identical re-put is the RFC 04 §1.2 refresh, not a transition.
    publication.send(doc.to_vec(), None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(ops.lock().unwrap().len(), 1, "a re-put is nothing");
    assert_eq!(mail.lock().unwrap().len(), 1);

    // 3. the tombstone → exactly one resolved, with no document fields.
    publication.retire().await.unwrap();
    let got = settled(&[Arc::clone(&ops), Arc::clone(&mail)], 2).await;
    for sink in &got {
        assert_eq!(sink.len(), 2);
        let n = &sink[1].notification;
        assert_eq!(
            n.state,
            CondState::Ok,
            "resolved is the established-clean pole"
        );
        assert_eq!(n.prior, Some(CondState::Firing));
        assert_eq!(
            n.severity, "warning",
            "the rule's default: a tombstone says nothing"
        );
        assert!(n.evidence.contains("resolved"), "{}", n.evidence);
        assert_eq!(n.rendering, RenderSource::KeyOnly);
        assert!(
            n.message
                .contains("payload not rendered: a tombstone carries no payload")
        );
    }

    // 4. a re-fire after the resolve is a firing again.
    publication.send(doc.to_vec(), None).await.unwrap();
    let got = settled(&[Arc::clone(&ops), Arc::clone(&mail)], 3).await;
    for sink in &got {
        assert_eq!(sink.len(), 3);
        assert_eq!(sink[2].notification.state, CondState::Firing);
        assert_eq!(sink[2].notification.prior, Some(CondState::Ok));
    }

    let _ = stop_tx.send(());
    let summary = tokio::time::timeout(util::SETTLE, engine)
        .await
        .expect("the engine stopped")
        .expect("no panic")
        .expect("a clean run");
    assert_eq!(summary.notices, 3);
    assert_eq!(
        summary.outgoing, 3,
        "one Outgoing per rule per genuine change"
    );
    assert_eq!(summary.delivered, 6, "two sinks, three changes");
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.dropped, 0);
}

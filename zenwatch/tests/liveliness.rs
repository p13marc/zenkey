//! `liveliness-gone <SEL>` against a real bus (#388): a producer's alive
//! token retired on peer A is one firing notification on peer B naming
//! the origin and producer; the token coming back is one `ok`. The token
//! already up when the observer joins is the baseline, never "came back".

mod util;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use zenkey_fleet::{BringUp, CondState, Fleet};
use zenwatch::config::DisciplineConfig;
use zenwatch::engine::{Engine, run_on};
use zenwatch::render::RenderConfig;
use zenwatch::rules::Rule;
use zenwatch::sinks::{NoticeKind, Outgoing, Sink};

const ALIVE: &str = "v1/h-3fa9c2d41b7e/state/netlink/alive";

async fn settled(capture: &Arc<Mutex<Vec<Outgoing>>>, n: usize) -> Vec<Outgoing> {
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    loop {
        let snapshot = capture.lock().unwrap().clone();
        if snapshot.len() >= n {
            return snapshot;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "waited {:?} for {n} deliveries; have {}",
            util::SETTLE,
            snapshot.len()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Until peer B can see the token on the roster — proof it was routed.
async fn token_visible(session: &zenoh::Session, expect: bool) {
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    loop {
        let replies = session
            .liveliness()
            .get("v1/**")
            .timeout(Duration::from_millis(500))
            .await
            .unwrap();
        let mut seen = false;
        while let Ok(r) = replies.recv_async().await {
            if r.result().is_ok_and(|s| s.key_expr().as_str() == ALIVE) {
                seen = true;
            }
        }
        if seen == expect {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "token visibility never became {expect}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_retired_alive_token_is_one_firing_and_its_return_is_one_ok() {
    let (a, b) = util::peer_pair().await;
    // The token is up before the observer joins: the roster replay on join
    // (history) is the baseline.
    let producer = BringUp::new(&a).alive(ALIVE).await.unwrap();

    let ops = Arc::new(Mutex::new(Vec::new()));
    let sinks = Arc::new(vec![Sink::capturing("ops", Arc::clone(&ops))]);
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    let observer = b.clone();
    let engine = tokio::spawn(async move {
        let rules = vec![
            Rule::from_config(&zenwatch::config::RuleConfig {
                name: "hosts-gone".into(),
                rule: "liveliness-gone v1/*/state/*/alive".into(),
                severity: Some("error".into()),
                labels: Default::default(),
                sinks: vec!["ops".into()],
                for_s: None,
            })
            .unwrap(),
        ];
        let render = RenderConfig::default();
        // No group window, no self-publication: this test is about the
        // rule, and every notification should leave on the next tick.
        let discipline = DisciplineConfig {
            group_window_s: 0.0,
            ..DisciplineConfig::default()
        };
        run_on(
            Engine {
                fleet: Fleet::new(&observer, ""),
                slices: None,
                timeout: Duration::from_secs(2),
                tick: Duration::from_secs(1),
                rules: &rules,
                sinks,
                render: &render,
                once: false,
                ready: Some(ready_tx),
                discipline: &discipline,
                state_file: None,
                state_max_entries: 4096,
                publish: false,
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
    token_visible(&b, true).await;
    // The history replay lands on the engine's broadcast a moment after the
    // subscriber is declared; give the baseline time to be recorded.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        ops.lock().unwrap().is_empty(),
        "a token already up is the baseline, not a return"
    );

    // Gone.
    producer.retire().await.unwrap();
    token_visible(&b, false).await;
    let got = settled(&ops, 1).await;
    let n = &got[0].notification;
    assert_eq!(n.state, CondState::Firing);
    assert_eq!(n.prior, Some(CondState::Ok), "the baseline was recorded");
    assert_eq!(n.kind, NoticeKind::Liveliness);
    assert_eq!(n.rule_kind, "liveliness-gone");
    assert_eq!(n.id, "hosts-gone:h-3fa9c2d41b7e/netlink");
    assert_eq!(n.severity, "error");
    assert!(
        n.evidence.contains("h-3fa9c2d41b7e/netlink"),
        "{}",
        n.evidence
    );
    assert!(n.evidence.contains("is gone"), "{}", n.evidence);
    assert_eq!(n.title, "error hosts-gone firing");
    assert!(
        n.message
            .contains("key: v1/h-3fa9c2d41b7e/state/netlink/alive")
    );

    // Back.
    let producer = BringUp::new(&a).alive(ALIVE).await.unwrap();
    let got = settled(&ops, 2).await;
    let n = &got[1].notification;
    assert_eq!(n.state, CondState::Ok);
    assert_eq!(n.prior, Some(CondState::Firing));
    assert_eq!(n.kind, NoticeKind::Resolved);
    assert!(n.evidence.contains("is back"), "{}", n.evidence);

    let _ = stop_tx.send(());
    let summary = tokio::time::timeout(util::SETTLE, engine)
        .await
        .expect("the engine stopped")
        .expect("no panic")
        .expect("a clean run");
    assert_eq!(summary.notices, 2);
    assert_eq!(summary.delivered, 2);
    producer.retire().await.unwrap();
}

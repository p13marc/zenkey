//! The scheduled doctor against a real zk2 deployment (#390; zk2's doctor
//! since #614, FL2): the tcgui pilot's frontend binds an optional
//! `scenario` role to `*/tc`, and nobody provides `tc.scenario.v1` — a
//! `binding-unsatisfied` finding, named by the **baseline** notification,
//! not paged on its own. A provider comes up; the next run yields exactly
//! one `resolved` naming the check; and every run publishes
//! `state/zenwatch/doctor` (v1's write side, until FL3), which a second
//! session parses as the served type with `outcome: ok`. `every_s` — the
//! tests-and-demos spelling — keeps it under half a minute.

mod util;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use util::engine::{self, Capture, Options};
use util::zk2;
use zenkey::ServiceBuilder;
use zenkey_fleet::CondState;
use zenoh::sample::SampleKind;
use zenwatch::config::DoctorConfig;
use zenwatch::publish::{DoctorOutcome, ZenwatchDoctor};
use zenwatch::sinks::NoticeKind;

/// What the explorer's subscriber saw on the doctor key: kind and body.
type Published = Arc<Mutex<Vec<(SampleKind, Vec<u8>)>>>;

const FINDING: &str = "doctor:binding-unsatisfied:ws-01/tcgui-frontend scenario";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unsatisfied_role_is_named_by_the_baseline_and_its_provider_is_one_resolved() {
    let (_router, ep) = zk2::router().await;
    let owners = zk2::client(&ep).await;
    let tool = zk2::client(&ep).await;

    // host-a/tc provides tc.netif.v1; the frontend binds `netif` and the
    // optional `scenario` to */tc, and nothing provides tc.scenario.v1.
    let _tc = zk2::bring_up(
        &owners,
        zk2::config("host-a/tc"),
        &[zk2::example("tcgui/tc.netif.v1")],
    )
    .await;
    let mut gui = ServiceBuilder::new(
        &owners,
        zk2::config("ws-01/tcgui-frontend")
            .bind("netif", &["*/tc"])
            .bind("scenario", &["*/tc"]),
    );
    gui.require("netif", "tc.netif.v1".parse().unwrap(), false);
    gui.require("scenario", "tc.scenario.v1".parse().unwrap(), true);
    let _gui = gui.start().await.expect("the frontend");
    zk2::tokens(&tool, "zk2/*/*/@zk/instance/*", 2).await;

    // The explorer's side: every doctor document the daemon publishes.
    let published: Published = Arc::new(Mutex::new(Vec::new()));
    let _sub = {
        let published = Arc::clone(&published);
        tool.declare_subscriber("v1/*/state/zenwatch/doctor")
            .callback(move |s| {
                published
                    .lock()
                    .unwrap()
                    .push((s.kind(), s.payload().to_bytes().to_vec()));
            })
            .await
            .unwrap()
    };

    let ops = Capture::default();
    let running = engine::start(
        zk2::daemon_bus(&ep, "").await,
        // One unrelated rule, so the daemon is a daemon; the doctor block
        // is what this test is about.
        vec![engine::rule("observer-drops", "dropped", "info")],
        ops.clone(),
        Options {
            tick: Duration::from_millis(250),
            publish: true,
            doctor: Some(DoctorConfig {
                every_h: None,
                every_s: Some(1.0),
                deep: false,
                sample: None,
                timeout_s: Some(1.0),
                sinks: vec!["ops".into()],
                severity_floor: None,
            }),
            ..Options::default()
        },
    )
    .await;

    // 1. The baseline: one info notification naming the finding — not a
    // page per finding.
    let got = engine::until(&ops, "the baseline", |s| {
        s.iter().any(|o| o.notification.id == "doctor:baseline")
    })
    .await;
    let baseline = &got
        .iter()
        .find(|o| o.notification.id == "doctor:baseline")
        .unwrap()
        .notification;
    assert_eq!(baseline.kind, NoticeKind::Doctor);
    assert_eq!(baseline.rule, "doctor");
    assert_eq!(baseline.severity, "info");
    assert_eq!(baseline.state, CondState::Firing);
    assert!(
        baseline.title.starts_with("doctor baseline: "),
        "{}",
        baseline.title
    );
    assert!(
        baseline
            .message
            .contains("binding-unsatisfied ws-01/tcgui-frontend scenario"),
        "the baseline names the unsatisfied role: {}",
        baseline.message
    );
    assert!(
        baseline.message.contains("presence: zk2/*/*/@zk/**"),
        "the coverage names the read: {}",
        baseline.message
    );
    assert!(
        !got.iter().any(|o| o.notification.id == FINDING),
        "a finding true since deployment is not news: {got:?}"
    );

    // 2. A provider comes up: the next run yields exactly one resolved.
    let _scenario = zk2::bring_up(
        &owners,
        zk2::config("host-b/tc"),
        &[zk2::example("tcgui/tc.scenario.v1")],
    )
    .await;
    let got = engine::until(&ops, "the resolve", |s| {
        s.iter()
            .any(|o| o.notification.kind == NoticeKind::Resolved)
    })
    .await;
    let resolved: Vec<_> = got
        .iter()
        .filter(|o| o.notification.kind == NoticeKind::Resolved)
        .collect();
    assert_eq!(resolved.len(), 1, "exactly one resolved: {got:?}");
    let n = &resolved[0].notification;
    assert_eq!(n.id, FINDING);
    assert_eq!(n.title, "binding-unsatisfied ws-01/tcgui-frontend scenario");
    assert_eq!(n.state, CondState::Ok);
    assert_eq!(n.prior, Some(CondState::Firing));
    assert_eq!(
        n.labels.get("check").map(String::as_str),
        Some("binding-unsatisfied")
    );
    assert!(
        n.message.contains("check: binding-unsatisfied (§3.2 R5)"),
        "{}",
        n.message
    );
    assert!(
        n.message.contains("gone since the run at "),
        "{}",
        n.message
    );

    // 3. The published report, read by a second session, parses as the
    // served type: the baseline run's document counts the finding, a later
    // one counts it fixed, and every one says `ok`.
    let docs: Vec<ZenwatchDoctor> = published
        .lock()
        .unwrap()
        .iter()
        .map(|(kind, body)| {
            assert_eq!(*kind, SampleKind::Put);
            serde_json::from_slice(body).expect("a ZenwatchDoctor")
        })
        .collect();
    assert!(docs.len() >= 2, "one document per run: {}", docs.len());
    assert!(
        docs.iter().all(|d| d.outcome == DoctorOutcome::Ok),
        "{docs:?}"
    );
    let first = &docs[0];
    assert!(first.findings >= 1);
    assert_eq!((first.new, first.fixed), (0, 0), "the baseline");
    assert_eq!(first.delta, None);
    assert_eq!(first.every_s, 1.0);
    let checks = first.report["checks"].as_array().expect("zk2's report");
    let binding = checks
        .iter()
        .find(|c| c["check"] == "binding-unsatisfied")
        .expect("the check");
    assert_eq!(binding["verdict"]["answer"], "established");
    let fixed = docs
        .iter()
        .find(|d| d.fixed >= 1)
        .expect("the run that saw the provider");
    assert_eq!(
        fixed.delta.as_ref().unwrap()["fixed"][0]["check"],
        "binding-unsatisfied"
    );

    let summary = running.stop().await;
    assert!(summary.doctor_runs >= 2, "{}", summary.doctor_runs);
}

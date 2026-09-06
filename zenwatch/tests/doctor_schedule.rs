//! The scheduled doctor against a real bus (#390): a producer serving an
//! `introspect` slice that disagrees with the daemon's registry is a
//! `slice-sync` finding — named by the **baseline** notification, not
//! paged on its own; when the producer comes to agree (the upgrade lands
//! on the fifth host), the next run yields exactly one `resolved`
//! naming the check; and every run publishes `state/zenwatch/doctor`,
//! which a second session parses as the served type with `outcome: ok`.
//! `every_s` — the tests-and-demos spelling — keeps it under ten seconds.
//! Ports are ephemeral (`util::peer_pair`).

mod util;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use zenkey_fleet::{CondState, Fleet};
use zenoh::sample::SampleKind;
use zenwatch::config::{DisciplineConfig, DoctorConfig, RuleConfig};
use zenwatch::engine::{Engine, run_on};
use zenwatch::publish::{DoctorOutcome, ZenwatchDoctor};
use zenwatch::render::RenderConfig;
use zenwatch::rules::Rule;
use zenwatch::sinks::{NoticeKind, Outgoing, Sink};

const ORIGIN: &str = "h-dddddddddddd";

/// What the producer served before the upgrade: version 1.0, one subject.
const SERVED_OLD: &str = r#"
[registry]
version = "1.0"
app = "t"
convention = 1
[producer]
name = "sysinfo"
[[subject]]
path = "health"
class = "state"
type = "Health"
"#;

/// The registry the daemon carries — and what the producer serves once it
/// has been upgraded.
const LOCAL: &str = r#"
[registry]
version = "2.0"
app = "t"
convention = 1
[producer]
name = "sysinfo"
[[subject]]
path = "health"
class = "state"
type = "Health"
[[subject]]
path = "cpu"
class = "telemetry"
type = "TelemetryPoint"
"#;

type Capture = Arc<Mutex<Vec<Outgoing>>>;
/// What the explorer's subscriber saw on the doctor key: kind and body.
type Published = Arc<Mutex<Vec<(SampleKind, Vec<u8>)>>>;

/// Until `pred` holds over what the sink captured (or SETTLE).
async fn until(capture: &Capture, what: &str, pred: impl Fn(&[Outgoing]) -> bool) -> Vec<Outgoing> {
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    loop {
        let snapshot = capture.lock().unwrap().clone();
        if pred(&snapshot) {
            return snapshot;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "waited {:?} for {what}; have {:?}",
            util::SETTLE,
            snapshot
                .iter()
                .map(|o| format!("{:?} {}", o.notification.kind, o.notification.id))
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_drifted_producer_is_named_by_the_baseline_and_its_upgrade_is_one_resolved() {
    let (a, b) = util::peer_pair().await;

    // The producer on peer A: alive, and answering introspect with whatever
    // `served` currently says — the seam that lets the upgrade land later.
    let served: Arc<Mutex<&'static str>> = Arc::new(Mutex::new(SERVED_OLD));
    let _token = a
        .liveliness()
        .declare_token(format!("v1/{ORIGIN}/state/sysinfo/alive"))
        .await
        .expect("token");
    let _queryable = {
        let served = Arc::clone(&served);
        a.declare_queryable(format!("v1/{ORIGIN}/@rpc/sysinfo/introspect"))
            .callback(move |query| {
                let body = *served.lock().unwrap();
                tokio::spawn(async move {
                    query
                        .reply(format!("v1/{ORIGIN}/@rpc/sysinfo/introspect"), body)
                        .await
                        .unwrap();
                });
            })
            .await
            .expect("queryable")
    };

    // The explorer's side: every doctor document the daemon publishes.
    let published: Published = Arc::new(Mutex::new(Vec::new()));
    let _sub = {
        let published = Arc::clone(&published);
        a.declare_subscriber("v1/*/state/zenwatch/doctor")
            .callback(move |s| {
                published
                    .lock()
                    .unwrap()
                    .push((s.kind(), s.payload().to_bytes().to_vec()));
            })
            .await
            .unwrap()
    };

    let ops: Capture = Arc::new(Mutex::new(Vec::new()));
    let sinks = Arc::new(vec![Sink::capturing("ops", Arc::clone(&ops))]);
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    let observer = b.clone();
    let engine = tokio::spawn(async move {
        // One unrelated rule, so the daemon is a daemon; the doctor block
        // is what this test is about.
        let rules = vec![
            Rule::from_config(&RuleConfig {
                name: "observer-drops".into(),
                rule: "dropped".into(),
                severity: Some("info".into()),
                labels: Default::default(),
                sinks: vec!["ops".into()],
                for_s: None,
            })
            .unwrap(),
        ];
        let doctor = DoctorConfig {
            every_h: None,
            every_s: Some(1.0),
            deep: false,
            sample: None,
            timeout_s: Some(2.0),
            sinks: vec!["ops".into()],
            severity_floor: None,
        };
        let render = RenderConfig::default();
        let discipline = DisciplineConfig {
            group_window_s: 0.0,
            ..DisciplineConfig::default()
        };
        let local = zenkey::parse_slice(LOCAL).expect("local slice");
        let slices = zenkey_fleet::SliceSet::from_slices(vec![local]);
        run_on(
            Engine {
                fleet: Fleet::new(&observer, ""),
                slices: Some(&slices),
                timeout: Duration::from_secs(2),
                tick: Duration::from_millis(250),
                rules: &rules,
                sinks,
                render: &render,
                once: false,
                ready: Some(ready_tx),
                discipline: &discipline,
                state_file: None,
                state_max_entries: 4096,
                publish: true,
                doctor: Some(&doctor),
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

    // 1. The baseline: one info notification naming the slice-sync
    // finding — not a page per finding.
    let got = until(&ops, "the baseline", |s| !s.is_empty()).await;
    let baseline = &got[0].notification;
    assert_eq!(baseline.id, "doctor:baseline");
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
            .contains(&format!("slice-sync {ORIGIN}/sysinfo")),
        "the baseline names the drifted producer: {}",
        baseline.message
    );
    assert!(
        baseline.message.contains("registry diff: asked"),
        "a registry was loaded, so the diff was asked: {}",
        baseline.message
    );
    assert_eq!(got[0].sinks, vec!["ops".to_string()]);
    assert!(
        !got.iter()
            .any(|o| o.notification.id.starts_with("doctor:slice-sync")),
        "a finding true since deployment is not news: {got:?}"
    );

    // 2. The upgrade lands: the producer now serves what the registry
    // declares. The next run yields exactly one resolved naming the check.
    *served.lock().unwrap() = LOCAL;
    let got = until(&ops, "the resolve", |s| {
        s.iter()
            .any(|o| o.notification.kind == NoticeKind::Resolved)
    })
    .await;
    let resolved: Vec<&Outgoing> = got
        .iter()
        .filter(|o| o.notification.kind == NoticeKind::Resolved)
        .collect();
    assert_eq!(resolved.len(), 1, "exactly one resolved: {got:?}");
    let n = &resolved[0].notification;
    assert_eq!(n.id, format!("doctor:slice-sync:{ORIGIN}/sysinfo"));
    assert_eq!(n.title, format!("slice-sync {ORIGIN}/sysinfo"));
    assert_eq!(n.state, CondState::Ok);
    assert_eq!(n.prior, Some(CondState::Firing));
    assert_eq!(
        n.labels.get("check").map(String::as_str),
        Some("slice-sync")
    );
    assert!(
        n.message.contains("gone since the run at "),
        "{}",
        n.message
    );
    assert!(
        !got.iter()
            .any(|o| o.notification.kind == NoticeKind::Unobservable),
        "every run happened: {got:?}"
    );

    // 3. The published report, read by a second session, parses as the
    // served type: the baseline run's document counts the finding, the
    // later one counts it fixed, and every one says `ok`.
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
    assert!(docs.iter().all(|d| d.error.is_none()));
    let first = &docs[0];
    assert!(first.findings >= 1);
    assert_eq!((first.new, first.fixed), (0, 0), "the baseline");
    assert_eq!(first.delta, None);
    assert_eq!(first.report_at.as_deref(), Some(first.ran_at.as_str()));
    assert_eq!(first.every_s, 1.0);
    assert!(
        first.report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["check"] == "slice-sync" && f["subject"] == format!("{ORIGIN}/sysinfo")),
        "{}",
        first.report
    );
    assert!(
        first.report.get("synced").is_some(),
        "a registry was loaded: the diff was asked, and says so"
    );
    let fixed = docs
        .iter()
        .find(|d| d.fixed >= 1)
        .expect("the run that saw the upgrade");
    assert_eq!(
        fixed.delta.as_ref().unwrap()["fixed"][0]["check"],
        "slice-sync"
    );
    assert!(
        !fixed.report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["check"] == "slice-sync"),
        "{}",
        fixed.report
    );

    let _ = stop_tx.send(());
    let summary = tokio::time::timeout(util::SETTLE, engine)
        .await
        .expect("the engine stopped")
        .expect("no panic")
        .expect("a clean run");
    assert!(summary.doctor_runs >= 2, "{}", summary.doctor_runs);
}

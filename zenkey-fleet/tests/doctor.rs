//! The doctor engine (#55) against a real bus: findings come out typed, with
//! their stable check ids and RFC citations — the same structs both
//! frontends render.
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::time::Duration;

use zenkey_fleet::{V1DoctorSpec, run_v1_doctor};

mod util;
use util::peer_pair;

const SERVED_SLICE: &str = r#"
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

const LOCAL_SLICE: &str = r#"
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

fn spec() -> V1DoctorSpec {
    V1DoctorSpec {
        deep: false,
        sample: None,
        timeout: Duration::from_secs(2),
        listen: None,
    }
}

/// A producer serving a slice that disagrees with the local registry yields
/// `slice-sync` error findings citing RFC 08 §6 — and the run's coverage
/// summary counts what was actually asked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_drifted_slice_is_a_sync_finding_with_its_citation() {
    let (a, b) = peer_pair().await;

    let _token = a
        .liveliness()
        .declare_token("v1/h-dddddddddddd/state/sysinfo/alive")
        .await
        .expect("token");
    let _queryable = a
        .declare_queryable("v1/h-dddddddddddd/@rpc/sysinfo/introspect")
        .callback(|query| {
            let q = query.clone();
            tokio::spawn(async move {
                q.reply("v1/h-dddddddddddd/@rpc/sysinfo/introspect", SERVED_SLICE)
                    .await
                    .unwrap();
            });
        })
        .await
        .expect("queryable");

    let local = zenkey::parse_slice(LOCAL_SLICE).expect("local slice");

    // Routing propagation is async; retry bounded until the roster sees the
    // token and the introspect answers.
    let report = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let report = run_v1_doctor(
                &zenkey_fleet::Fleet::new(&b, ""),
                Some(&zenkey_fleet::SliceSet::from_slices(vec![local.clone()])),
                &spec(),
            )
            .await
            .expect("run_v1_doctor");
            if report.live_producers >= 1 && report.introspect_answered >= 1 {
                break report;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("fleet should become visible within 10s");

    let sync: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check == zenkey_fleet::report::V1CheckId::SliceSync)
        .collect();
    assert!(
        !sync.is_empty(),
        "a version/subject drift must yield slice-sync findings, got: {:?}",
        report.findings
    );
    assert!(
        sync.iter()
            .all(|f| f.citation.as_deref() == Some("RFC 08 §6")),
        "slice-sync findings carry their normative citation"
    );
    assert!(
        sync.iter().all(|f| f.subject == "h-dddddddddddd/sysinfo"),
        "findings attribute the origin and producer"
    );
}

/// A live token whose producer answers no introspect is an
/// `introspect-coverage` error — alive ⇒ callable (RFC 04 §5), never a
/// boot-race excuse.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mute_live_producer_is_a_coverage_finding() {
    let (a, b) = peer_pair().await;

    let _token = a
        .liveliness()
        .declare_token("v1/h-eeeeeeeeeeee/state/mute/alive")
        .await
        .expect("token");

    let report = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let report = run_v1_doctor(&zenkey_fleet::Fleet::new(&b, ""), None, &spec())
                .await
                .expect("run_v1_doctor");
            if report.live_producers >= 1 {
                break report;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the token should become visible within 10s");

    assert!(
        report.findings.iter().any(|f| f.check
            == zenkey_fleet::report::V1CheckId::IntrospectCoverage
            && f.citation.as_deref() == Some("RFC 04 §5")),
        "a mute live producer must be a coverage finding, got: {:?}",
        report.findings
    );
    // A token on the roster is something in scope: the run is a verdict.
    assert_eq!(report.unobservable, None);
}

/// #510: a bus with nothing on it — no token, no router answering, no
/// answer of any kind — is a run that judged nothing. The report says so
/// with its reason, and its judgement is `Unobservable` under every
/// threshold: the coverage check compares 0 with 0 and must not read as a
/// healthy fleet.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_empty_bus_is_a_run_that_judged_nothing() {
    use zenkey_fleet::report::DoctorSeverity;
    let (_a, b) = peer_pair().await;
    let report = run_v1_doctor(&zenkey_fleet::Fleet::new(&b, "acme"), None, &spec())
        .await
        .expect("run_v1_doctor");
    assert_eq!((report.live_producers, report.routers), (0, 0));
    let why = report.unobservable.as_deref().expect("judged nothing");
    assert!(
        why.contains("nothing in scope") && why.contains("\"acme\""),
        "{why}"
    );
    for threshold in [None, Some(DoctorSeverity::Error)] {
        assert!(
            report.judgement(threshold).is_unobservable(),
            "{threshold:?}"
        );
    }
}

/// A live producer that *answers* introspect with a slice this build cannot
/// read is not mute (#491): with no `--registry`, the wildcard sweep counts
/// it as answered — no `introspect-coverage` finding — and files the same
/// `slice-parse` the served-vs-declared diff would, naming the encoding
/// (RFC 08 §6, v1.44; RFC 13 §3 O4).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreadable_introspect_is_a_parse_finding_not_silence() {
    let (a, b) = peer_pair().await;

    let _token = a
        .liveliness()
        .declare_token("v1/h-ffffffffffff/state/sysinfo/alive")
        .await
        .expect("token");
    let _queryable = a
        .declare_queryable("v1/h-ffffffffffff/@rpc/sysinfo/introspect")
        .callback(|query| {
            let q = query.clone();
            tokio::spawn(async move {
                q.reply("v1/h-ffffffffffff/@rpc/sysinfo/introspect", SERVED_SLICE)
                    .encoding("application/json")
                    .await
                    .unwrap();
            });
        })
        .await
        .expect("queryable");

    let report = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let report = run_v1_doctor(&zenkey_fleet::Fleet::new(&b, ""), None, &spec())
                .await
                .expect("run_v1_doctor");
            if report.live_producers >= 1 && report.introspect_answered >= 1 {
                break report;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the token and the reply should become visible within 10s");

    use zenkey_fleet::report::V1CheckId;
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.check == V1CheckId::IntrospectCoverage),
        "it answered, so it is not a coverage finding: {:?}",
        report.findings
    );
    let parse: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check == V1CheckId::SliceParse)
        .collect();
    assert_eq!(parse.len(), 1, "{:?}", report.findings);
    assert_eq!(parse[0].subject, "h-ffffffffffff/sysinfo");
    assert!(
        parse[0].evidence.contains("application/json"),
        "{}",
        parse[0].evidence
    );
}

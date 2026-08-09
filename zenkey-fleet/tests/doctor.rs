//! The doctor engine (#55) against a real bus: findings come out typed, with
//! their stable check ids and RFC citations — the same structs both
//! frontends render. Ports 7491-7492.

use std::time::Duration;

use zenkey_fleet::{DoctorSpec, run_doctor};

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

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

fn spec() -> DoctorSpec {
    DoctorSpec {
        deep: false,
        sample: None,
        timeout: Duration::from_secs(2),
    }
}

/// A producer serving a slice that disagrees with the local registry yields
/// `slice-sync` error findings citing RFC 08 §6 — and the run's coverage
/// summary counts what was actually asked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_drifted_slice_is_a_sync_finding_with_its_citation() {
    let (a, b) = peer_pair(7491).await;

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
            let report = run_doctor(&b, "", std::slice::from_ref(&local), &spec())
                .await
                .expect("run_doctor");
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
        .filter(|f| f.check == "slice-sync")
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
    let (a, b) = peer_pair(7492).await;

    let _token = a
        .liveliness()
        .declare_token("v1/h-eeeeeeeeeeee/state/mute/alive")
        .await
        .expect("token");

    let report = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let report = run_doctor(&b, "", &[], &spec()).await.expect("run_doctor");
            if report.live_producers >= 1 {
                break report;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the token should become visible within 10s");

    assert!(
        report
            .findings
            .iter()
            .any(|f| f.check == "introspect-coverage"
                && f.citation.as_deref() == Some("RFC 04 §5")),
        "a mute live producer must be a coverage finding, got: {:?}",
        report.findings
    );
}

//! The doctor engine (#55) against a real bus: findings come out typed, with
//! their stable check ids and RFC citations — the same structs both
//! frontends render.
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::time::Duration;

use zenkey_fleet::{DoctorSpec, run_doctor};

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

fn spec() -> DoctorSpec {
    DoctorSpec {
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
            let report = run_doctor(
                &zenkey_fleet::Fleet::new(&b, ""),
                Some(&zenkey_fleet::SliceSet::from_slices(vec![local.clone()])),
                &spec(),
            )
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
        .filter(|f| f.check == zenkey_fleet::report::CheckId::SliceSync)
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
            let report = run_doctor(&zenkey_fleet::Fleet::new(&b, ""), None, &spec())
                .await
                .expect("run_doctor");
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
            == zenkey_fleet::report::CheckId::IntrospectCoverage
            && f.citation.as_deref() == Some("RFC 04 §5")),
        "a mute live producer must be a coverage finding, got: {:?}",
        report.findings
    );
}

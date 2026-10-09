//! v1's registry checks (#55), the ones `check conform` projects, against a
//! real bus: the served-vs-declared slice diff, with its stable check ids and
//! RFC citations. They were the doctor's until #612's FJ6; zk2's doctor has
//! its own suite (`tests/zk2_doctor.rs`).
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::time::Duration;

use zenkey_fleet::judge::registry_checks::{RegistrySpec, run as registry_checks};

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

fn spec() -> RegistrySpec {
    RegistrySpec {
        deep: false,
        sample: None,
        timeout: Duration::from_secs(2),
        listen: None,
    }
}

/// A producer serving a slice that disagrees with the local registry yields
/// `slice-sync` error findings citing RFC 08 §6.
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
            let report = registry_checks(
                &zenkey_fleet::Fleet::new(&b, ""),
                Some(&zenkey_fleet::SliceSet::from_slices(vec![local.clone()])),
                &spec(),
            )
            .await
            .expect("the registry checks");
            if report
                .findings
                .iter()
                .any(|f| f.check == zenkey_fleet::report::V1CheckId::SliceSync)
            {
                break report;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the introspect should answer within 10s");

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

/// A producer that *answers* introspect with a slice this build cannot read
/// is not silent (#491): with no registry given, the wildcard sweep files
/// the same `slice-parse` the served-vs-declared diff would, naming the
/// encoding (RFC 08 §6, v1.44; RFC 13 §3 O4).
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
            let report = registry_checks(&zenkey_fleet::Fleet::new(&b, ""), None, &spec())
                .await
                .expect("the registry checks");
            if report
                .findings
                .iter()
                .any(|f| f.check == zenkey_fleet::report::V1CheckId::SliceParse)
            {
                break report;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the reply should become visible within 10s");

    use zenkey_fleet::report::V1CheckId;
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

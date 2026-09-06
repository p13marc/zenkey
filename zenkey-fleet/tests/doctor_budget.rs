//! `doctor --deep` judges a producer's declared `[budget]` (#391, RFC 08 §2
//! v1.32, RFC 13 §3) against the `self_stats` on its health document (RFC
//! 04 §1.2), on a real bus: a resident set over `rss_mb` is a
//! `budget-exceeded` finding naming its origin, a table over its bound is
//! one too, a document that does not say how big the producer is stays an
//! unobservability rather than a pass, and a slice that declares no budget
//! is never asked. Under `--deep` because a health fetch costs the data
//! plane (RFC 13 §3's frugality note).
//!
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::time::Duration;

use zenkey_fleet::report::{CheckId, DoctorFinding, DoctorReport, DoctorSeverity};
use zenkey_fleet::{DoctorSpec, Fleet, GetOpts, declare_responder, fleet_get, run_doctor};

mod util;
use util::peer_pair;

const HEALTH: &str = "v1/h-abababababab/state/demo/health";

/// The producer under test declares a 64 MiB budget and one bounded table…
const BUDGETED_SLICE: &str = r#"
[registry]
version = "1.0"
app = "t"
convention = 1
[producer]
name = "demo"
[budget]
rss_mb = 64
[[budget.tables]]
name = "flows"
max_entries = 65536
[[subject]]
path = "health"
class = "state"
type = "Health"
"#;

/// …and the same producer declaring nothing about its cost.
const UNBUDGETED_SLICE: &str = r#"
[registry]
version = "1.0"
app = "t"
convention = 1
[producer]
name = "demo"
[[subject]]
path = "health"
class = "state"
type = "Health"
"#;

fn spec() -> DoctorSpec {
    DoctorSpec {
        deep: true,
        sample: None,
        timeout: Duration::from_millis(500),
        listen: None,
    }
}

/// Serve `body` as the health document, answering every ask until the test
/// ends, and settle until a GET through the chokepoint sees it — routing
/// propagation is async, and "no health document answered" must mean the
/// producer, not the fixture.
async fn serve_health(
    server: &zenoh::Session,
    client: &zenoh::Session,
    body: &'static str,
) -> tokio::task::JoinHandle<()> {
    let responder = declare_responder(
        server,
        HEALTH,
        body.as_bytes().to_vec(),
        Some("application/json"),
        false,
    )
    .await
    .expect("declare the health responder");
    let task = tokio::spawn(async move {
        while let Some(query) = responder.next().await {
            responder.answer(query).await;
        }
    });
    tokio::time::timeout(util::SETTLE, async {
        loop {
            let answers = fleet_get(
                &Fleet::new(client, ""),
                HEALTH,
                &GetOpts::new(Duration::from_millis(500)),
            )
            .await
            .expect("get");
            if !answers.is_empty() {
                break;
            }
        }
    })
    .await
    .expect("the health responder becomes routable");
    task
}

async fn doctor(client: &zenoh::Session, slice: &str) -> DoctorReport {
    let local = zenkey::parse_slice(slice).expect("slice");
    run_doctor(
        &Fleet::new(client, ""),
        Some(&zenkey_fleet::SliceSet::from_slices(vec![local])),
        &spec(),
    )
    .await
    .expect("run_doctor")
}

fn budget_findings(report: &DoctorReport) -> Vec<&DoctorFinding> {
    report
        .findings
        .iter()
        .filter(|f| f.check == CheckId::BudgetExceeded)
        .collect()
}

/// 100 MiB resident against a 64 MiB budget: one Error, per origin, with
/// both numbers, citing the declaration.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_producer_over_its_declared_rss_budget_is_a_finding() {
    let (a, b) = peer_pair().await;
    let t = serve_health(
        &a,
        &b,
        r#"{"status":"Healthy","self_stats":{"rss_bytes":104857600,"tables":[{"name":"flows","entries":10}]}}"#,
    )
    .await;

    let report = doctor(&b, BUDGETED_SLICE).await;
    t.abort();

    let found = budget_findings(&report);
    assert_eq!(found.len(), 1, "{:?}", report.findings);
    let f = found[0];
    assert_eq!(f.severity, DoctorSeverity::Error);
    assert_eq!(f.subject, "h-abababababab/demo", "per origin");
    assert!(f.evidence.contains("100.0 MiB"), "{}", f.evidence);
    assert!(f.evidence.contains("64 MiB"), "{}", f.evidence);
    assert_eq!(f.citation.as_deref(), Some("RFC 08 §2"));
}

/// 10 MiB resident and a table within its bound: Established(yes) for the
/// sample read, which is no finding of any severity.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_producer_under_budget_is_no_finding() {
    let (a, b) = peer_pair().await;
    let t = serve_health(
        &a,
        &b,
        r#"{"status":"Healthy","self_stats":{"rss_bytes":10485760,"budget_bytes":67108864,"tables":[{"name":"flows","entries":4096,"bytes":1048576}]}}"#,
    )
    .await;

    let report = doctor(&b, BUDGETED_SLICE).await;
    t.abort();

    assert!(
        budget_findings(&report).is_empty(),
        "within budget is clean: {:?}",
        report.findings
    );
}

/// The resident set is fine and the `flows` table holds more entries than
/// it declared it may: an Error naming the table and both numbers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_table_over_its_bound_is_a_finding() {
    let (a, b) = peer_pair().await;
    let t = serve_health(
        &a,
        &b,
        r#"{"status":"Healthy","self_stats":{"rss_bytes":10485760,"tables":[{"name":"flows","entries":70000}]}}"#,
    )
    .await;

    let report = doctor(&b, BUDGETED_SLICE).await;
    t.abort();

    let found = budget_findings(&report);
    assert_eq!(found.len(), 1, "{:?}", report.findings);
    let f = found[0];
    assert_eq!(f.severity, DoctorSeverity::Error);
    assert_eq!(f.subject, "h-abababababab/demo");
    assert!(f.evidence.contains("`flows`"), "{}", f.evidence);
    assert!(f.evidence.contains("70000"), "{}", f.evidence);
    assert!(f.evidence.contains("65536"), "{}", f.evidence);
}

/// A health document with no `self_stats` is an older producer, not a
/// wrong one: unobservable, and the reason is the finding an operator
/// wants — one Warning, never an Error, never a pass (RFC 13 §3).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_health_document_without_self_stats_is_unobservable_not_a_verdict() {
    let (a, b) = peer_pair().await;
    let t = serve_health(&a, &b, r#"{"status":"Healthy"}"#).await;

    let report = doctor(&b, BUDGETED_SLICE).await;
    t.abort();

    let found = budget_findings(&report);
    assert_eq!(found.len(), 1, "{:?}", report.findings);
    let f = found[0];
    assert_eq!(f.severity, DoctorSeverity::Warning);
    assert_eq!(f.subject, "h-abababababab/demo");
    assert!(
        f.evidence.contains("does not say how big it is"),
        "{}",
        f.evidence
    );
    assert_eq!(f.citation.as_deref(), Some("RFC 04 §1.2"));
}

/// The same over-budget document under a slice that declares no `[budget]`
/// is not asked — and not asked is never answered, at any severity (RFC 13
/// §3, RFC 09 §5.1 O4).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slice_without_a_budget_is_never_asked() {
    let (a, b) = peer_pair().await;
    let t = serve_health(
        &a,
        &b,
        r#"{"status":"Healthy","self_stats":{"rss_bytes":104857600}}"#,
    )
    .await;

    let report = doctor(&b, UNBUDGETED_SLICE).await;
    t.abort();

    assert!(
        budget_findings(&report).is_empty(),
        "an undeclared budget is not asked: {:?}",
        report.findings
    );
}

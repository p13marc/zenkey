//! The describe sweep keeps the origin that answered (#398) — against real
//! zenoh, because the origin comes from the reply's own key and nothing
//! below the bus can prove that.
//!
//! The sibling of `registry_sweep.rs`, one plane over, and the worse plane to
//! lose attribution on. A registry disagreement at least names the *producer*
//! to go and read. A schema disagreement that names only a producer says a
//! type has two identities somewhere in the fleet and gives nobody a host to
//! go and look at — and it is the more likely disagreement, because a schema
//! hash changes on any field addition, so a half-rolled-out sensor produces
//! one by default.
//!
//! Self-contained: two in-process peers, ephemeral ports (`util::peer_pair`),
//! no router, no scouting.

use std::time::Duration;

use zenkey::schema::{SchemaSet, TypeSchema};
use zenkey_fleet::report::CheckId;
use zenkey_fleet::{DoctorSpec, run_doctor};

mod util;
use util::peer_pair;

const OLD_HOST: &str = "h-aaaaaaaaaaaa";
const NEW_HOST: &str = "h-bbbbbbbbbbbb";

/// The one producer both hosts run.
const SLICE: &str = r#"
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

/// Two hosts mid-rollout: the same type name, one field apart, so the two
/// sets hash differently.
fn schema_json(with_extra_field: bool) -> String {
    let mut properties = serde_json::json!({"up": {"type": "boolean"}});
    if with_extra_field {
        properties["degraded"] = serde_json::json!({"type": "boolean"});
    }
    SchemaSet::builder("t")
        .entry(
            "Health",
            TypeSchema::json_schema(serde_json::json!({
                "type": "object",
                "properties": properties,
            })),
        )
        .build()
        .to_json()
}

/// Serve `describe` for one origin, replying on that origin's own concrete
/// key — which is what the attribution reads.
async fn serve_describe(
    session: &zenoh::Session,
    origin: &str,
    body: String,
) -> zenoh::query::Queryable<()> {
    let key = format!("v1/{origin}/@rpc/sysinfo/describe");
    let reply_key = key.clone();
    session
        .declare_queryable(&key)
        .callback(move |query| {
            let q = query.clone();
            let reply_key = reply_key.clone();
            let body = body.clone();
            tokio::spawn(async move {
                q.reply(reply_key, body).await.unwrap();
            });
        })
        .await
        .expect("describe queryable")
}

fn spec() -> DoctorSpec {
    DoctorSpec {
        deep: false,
        sample: None,
        timeout: Duration::from_secs(2),
        listen: None,
    }
}

/// The acceptance case: two hosts serve one producer with different schema
/// hashes, and `doctor` names **which host serves which identity** rather
/// than reporting one arbitrary answer as the fleet's.
///
/// Before #398 this run produced no finding at all: the sweep kept the first
/// parseable reply, so one producer meant one claim, and drift needs two to
/// compare. A fleet mid-rollout read as agreeing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_schema_disagreement_names_which_host_serves_which_identity() {
    let (a, b) = peer_pair().await;
    let _old = serve_describe(&a, OLD_HOST, schema_json(false)).await;
    let _new = serve_describe(&a, NEW_HOST, schema_json(true)).await;

    let local = zenkey::parse_slice(SLICE).expect("local slice");
    let locals = zenkey_fleet::SliceSet::from_slices(vec![local]);
    let fleet = zenkey_fleet::Fleet::new(&b, "");

    // Routing propagation is async; retry bounded until the finding appears.
    let drift = tokio::time::timeout(util::SETTLE, async {
        loop {
            let report = run_doctor(&fleet, Some(&locals), &spec())
                .await
                .expect("doctor");
            let drift: Vec<_> = report
                .findings
                .iter()
                .filter(|f| f.check == CheckId::SchemaDrift)
                .cloned()
                .collect();
            if !drift.is_empty() {
                break drift;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("both hosts should answer, and disagree, within the settle window");

    assert_eq!(drift.len(), 1, "{drift:#?}");
    let evidence = &drift[0].evidence;
    assert!(
        evidence.contains(OLD_HOST) && evidence.contains(NEW_HOST),
        "both hosts must be named — a producer name alone gives nobody to ssh to: {evidence}"
    );
    assert_eq!(
        drift[0].subject, "Health",
        "the subject stays the type name: the GUI keys its deltas on (check, subject)"
    );
    assert_eq!(drift[0].citation.as_deref(), Some("RFC 08 §7"));
}

/// One host answering is not a disagreement, and the producer is still
/// described: the SHOULD is met, and nothing is reported.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_single_host_serving_a_schema_is_no_finding() {
    let (a, b) = peer_pair().await;
    let _only = serve_describe(&a, OLD_HOST, schema_json(false)).await;

    let local = zenkey::parse_slice(SLICE).expect("local slice");
    let locals = zenkey_fleet::SliceSet::from_slices(vec![local]);
    let fleet = zenkey_fleet::Fleet::new(&b, "");

    let report = tokio::time::timeout(util::SETTLE, async {
        loop {
            let report = run_doctor(&fleet, Some(&locals), &spec())
                .await
                .expect("doctor");
            if report.describe_served >= 1 {
                break report;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the one host should answer within the settle window");

    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.check == CheckId::SchemaDrift),
        "one claim is nothing to compare: {:#?}",
        report.findings
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.check == CheckId::DescribeMissing),
        "the producer *is* described: {:#?}",
        report.findings
    );
}

/// The sweep helper itself (#410), which `interface show --schema` now reads
/// instead of a `SchemaStore`: two hosts serving one producer at two hashes
/// come back as two attributed answers with distinct origins, the
/// per-producer fold keeps exactly one, and `schema_drift` over the answers
/// is the disagreement — the same verdict `doctor` reports above, from the
/// same evidence.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_sweep_helper_keeps_one_answer_per_origin() {
    let (a, b) = peer_pair().await;
    let _old = serve_describe(&a, OLD_HOST, schema_json(false)).await;
    let _new = serve_describe(&a, NEW_HOST, schema_json(true)).await;

    let local = zenkey::parse_slice(SLICE).expect("local slice");
    let locals = zenkey_fleet::SliceSet::from_slices(vec![local]);
    let fleet = zenkey_fleet::Fleet::new(&b, "");

    // Routing propagation is async; retry bounded until both hosts answer.
    let sweep = tokio::time::timeout(util::SETTLE, async {
        loop {
            let sweep = zenkey_fleet::describe_sweep(&fleet, &locals, Duration::from_secs(2))
                .await
                .expect("sweep");
            if sweep.answers.len() >= 2 {
                break sweep;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("both hosts should answer within the settle window");

    assert_eq!(sweep.answers.len(), 2, "{:#?}", sweep.answers);
    let origins: std::collections::BTreeSet<&str> =
        sweep.answers.iter().map(|d| d.origin.as_str()).collect();
    assert_eq!(
        origins,
        [OLD_HOST, NEW_HOST].into_iter().collect(),
        "one answer per origin, attributed by the reply's own key"
    );
    assert!(
        sweep.answers.iter().all(|d| d.producer == "sysinfo"),
        "both answers are for the one producer asked"
    );
    assert!(
        sweep.undescribed.is_empty(),
        "a producer two hosts answered for is described: {:?}",
        sweep.undescribed
    );
    let first = sweep.first_per_producer();
    assert_eq!(first.len(), 1, "the fold keeps one set per producer");
    assert_eq!(first[0].0, "sysinfo");

    let drift = zenkey_fleet::schema_drift(&sweep.answers);
    assert_eq!(drift.len(), 1, "{drift:#?}");
    assert_eq!(drift[0].type_name, "Health");
    assert_eq!(
        drift[0].verdict,
        zenkey_fleet::report::DriftVerdict::Disagree
    );
    let named: std::collections::BTreeSet<&str> =
        drift[0].servers.iter().map(|s| s.origin.as_str()).collect();
    assert_eq!(
        named, origins,
        "the verdict names the same hosts the sweep heard"
    );
}

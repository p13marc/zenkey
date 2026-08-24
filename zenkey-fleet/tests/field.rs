//! Field intelligence (#223) against a real bus — the acceptance case: a
//! fixture stream with one frozen field is flagged `field-stuck` while
//! `expect --valid-payload` and the rate floor both stay green, and the
//! seen-then-gone / never-declared paths become their findings.
//!
//! The fixture publishers send continuously through each window, so no
//! settle is needed beyond the publisher's matching badge (a fixture that
//! publishes into the void tests the void).
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::time::Duration;

use zenkey::qos::QosProfile;
use zenkey::schema::{SchemaSet, TypeSchema};
use zenkey_fleet::report::ExpectVerdict;
use zenkey_fleet::{ExpectSpec, FieldSpec, declare_publication, run_expect, run_field};

mod util;
use util::peer_pair;

const ORIGIN: &str = "h-adadadadadad";
const KEY: &str = "v1/h-adadadadadad/state/demo/health";

const SLICE: &str = r#"
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
ttl_s = 1
"#;

fn health_schema() -> SchemaSet {
    SchemaSet::builder("t")
        .entry(
            "Health",
            TypeSchema::json_schema(serde_json::json!({
                "type": "object",
                "properties": {
                    "temperature_c": {"type": "number"},
                    "seq": {"type": "number"},
                    "opt": {"type": "number"},
                },
            })),
        )
        .build()
}

/// Serve `@rpc/demo/describe` for the fixture origin, so validity is
/// checkable and `field-new` has a declared surface to judge against.
async fn serve_describe(session: &zenoh::Session) -> zenoh::query::Queryable<()> {
    let key = format!("v1/{ORIGIN}/@rpc/demo/describe");
    let payload = health_schema().to_json();
    let reply_key = key.clone();
    session
        .declare_queryable(&key)
        .callback(move |query| {
            let q = query.clone();
            let reply_key = reply_key.clone();
            let payload = payload.clone();
            tokio::spawn(async move {
                q.reply(reply_key, payload).await.unwrap();
            });
        })
        .await
        .expect("describe queryable")
}

/// Publish one JSON document per 100ms tick until dropped, each body built
/// from the tick counter.
fn keep_publishing(
    publication: zenkey_fleet::Publication,
    body: impl Fn(u64) -> serde_json::Value + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        for i in 0..200u64 {
            let bytes = serde_json::to_vec(&body(i)).expect("fixture body");
            if publication.send(bytes, None).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
}

fn store_of() -> zenkey_fleet::decode::SchemaStore {
    zenkey_fleet::decode::SchemaStore::new("", Duration::from_millis(500))
}

fn slices_of() -> zenkey_fleet::SliceSet {
    zenkey_fleet::SliceSet::from_slices(vec![zenkey::parse_slice(SLICE).expect("fixture slice")])
}

/// The #223 acceptance case: `temperature_c` frozen while `seq` moves — the
/// frozen field is flagged `field-stuck` (window and ttl stated), the moving
/// one is not, and the same stream passes `expect --valid-payload` with a
/// rate floor: exactly the failure mode every per-sample check renders green.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_frozen_field_is_flagged_while_validity_and_rate_stay_green() {
    let (a, b) = peer_pair().await;
    let slices = slices_of();
    let _describe = serve_describe(&a).await;

    let publication = declare_publication(&a, KEY, QosProfile::Transition, None)
        .await
        .expect("declare");
    let matching = publication.matching_events().await.expect("events");

    let field = tokio::spawn({
        let b = b.clone();
        let slices = slices.clone();
        async move {
            let spec = FieldSpec {
                selector: KEY.to_string(),
                window: Duration::from_secs(4),
                max_paths: 64,
            };
            run_field(
                &zenkey_fleet::Fleet::new(&b, ""),
                Some(&slices),
                &store_of(),
                &spec,
            )
            .await
        }
    });
    // The field window's subscriber raises the badge; then publish into it.
    assert!(
        tokio::time::timeout(Duration::from_secs(5), matching.recv())
            .await
            .expect("matching within 5s")
            .expect("listener alive")
    );
    let publisher = keep_publishing(
        publication,
        |i| serde_json::json!({"temperature_c": 21.5, "seq": i}),
    );

    let report = field.await.expect("join").expect("run_field");
    assert!(report.samples > 20, "the window saw the stream: {report:?}");
    assert_eq!(report.dropped, 0);
    assert_eq!(report.paths, 2);
    assert_eq!(report.paths_dropped, 0, "nothing hit the bound");

    let stuck: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check == "field-stuck")
        .collect();
    assert_eq!(stuck.len(), 1, "{:?}", report.findings);
    assert_eq!(stuck[0].subject, format!("{KEY} · temperature_c"));
    assert!(
        stuck[0].evidence.contains("ttl_s 1s"),
        "the ttl it is long relative to is stated: {}",
        stuck[0].evidence
    );
    assert!(
        stuck[0].evidence.contains("not a verdict"),
        "{}",
        stuck[0].evidence
    );
    assert!(
        report
            .findings
            .iter()
            .all(|f| !(f.check == "field-stuck" && f.subject.ends_with("· seq"))),
        "the moving field is not stuck"
    );
    assert!(
        report.findings.iter().all(|f| f.check != "field-new"),
        "both paths are declared by the served schema: {:?}",
        report.findings
    );

    // The same stream is green to every per-sample check: valid payloads at
    // a healthy rate — which is why #223 exists.
    let expect = ExpectSpec {
        selector: KEY.to_string(),
        within: Duration::from_secs(2),
        valid_payload: true,
        rate_min: Some(1.0),
        ..ExpectSpec::default()
    };
    let report = run_expect(
        &zenkey_fleet::Fleet::new(&b, ""),
        Some(&slices),
        &store_of(),
        &expect,
    )
    .await
    .expect("run_expect");
    assert_eq!(
        report.verdict,
        ExpectVerdict::Met,
        "validity and rate stay green over the frozen field: {:?}",
        report.unmet
    );

    publisher.abort();
}

/// The other two findings on one stream: `opt` present early then absent —
/// `field-vanished`, invisible to validation because the schema declares it
/// optional — and `extra`, a path the served schema never declared —
/// `field-new`, schema drift at field granularity.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vanished_and_undeclared_paths_become_their_findings() {
    let (a, b) = peer_pair().await;
    let slices = slices_of();
    let _describe = serve_describe(&a).await;

    let publication = declare_publication(&a, KEY, QosProfile::Transition, None)
        .await
        .expect("declare");
    let matching = publication.matching_events().await.expect("events");

    let field = tokio::spawn({
        let b = b.clone();
        let slices = slices.clone();
        async move {
            let spec = FieldSpec {
                selector: KEY.to_string(),
                window: Duration::from_secs(3),
                max_paths: 64,
            };
            run_field(
                &zenkey_fleet::Fleet::new(&b, ""),
                Some(&slices),
                &store_of(),
                &spec,
            )
            .await
        }
    });
    assert!(
        tokio::time::timeout(Duration::from_secs(5), matching.recv())
            .await
            .expect("matching within 5s")
            .expect("listener alive")
    );
    let publisher = keep_publishing(publication, |i| {
        if i < 5 {
            serde_json::json!({"seq": i, "opt": 1})
        } else {
            serde_json::json!({"seq": i, "extra": "x"})
        }
    });

    let report = field.await.expect("join").expect("run_field");
    publisher.abort();

    let vanished: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check == "field-vanished")
        .collect();
    assert_eq!(vanished.len(), 1, "{:?}", report.findings);
    assert_eq!(vanished[0].subject, format!("{KEY} · opt"));
    assert!(
        vanished[0].evidence.contains("absent from the last"),
        "{}",
        vanished[0].evidence
    );

    let new: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check == "field-new")
        .collect();
    assert_eq!(new.len(), 1, "{:?}", report.findings);
    assert_eq!(new[0].subject, format!("{KEY} · extra"));
    assert!(new[0].evidence.contains("Health"), "{}", new[0].evidence);
    // `seq` moved and stayed declared: never a finding.
    assert!(
        report
            .findings
            .iter()
            .all(|f| !f.subject.ends_with("· seq")),
        "{:?}",
        report.findings
    );
}

/// The doctor half: `--for` runs the same judges, so the frozen field
/// is a `field-stuck` finding under the stable check-id vocabulary — which is
/// what makes `zenctl watchdog --rule 'doctor field-stuck'` a thing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_doctor_listen_phase_flags_the_frozen_field() {
    let (a, b) = peer_pair().await;
    let local = zenkey::parse_slice(SLICE).expect("fixture slice");

    let publication = declare_publication(&a, KEY, QosProfile::Transition, None)
        .await
        .expect("declare");
    let publisher = keep_publishing(
        publication,
        |i| serde_json::json!({"temperature_c": 21.5, "seq": i}),
    );

    let report = zenkey_fleet::run_doctor(
        &zenkey_fleet::Fleet::new(&b, ""),
        Some(&zenkey_fleet::SliceSet::from_slices(vec![local.clone()])),
        &zenkey_fleet::DoctorSpec {
            deep: false,
            sample: None,
            timeout: Duration::from_millis(500),
            listen: Some(Duration::from_secs(4)),
        },
    )
    .await
    .expect("run_doctor");
    publisher.abort();

    let stuck: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check == "field-stuck")
        .collect();
    assert_eq!(stuck.len(), 1, "{:?}", report.findings);
    assert_eq!(stuck[0].subject, format!("{KEY} · temperature_c"));
    // Nothing served a describe here, so `field-new` has no declared surface
    // to judge against — unjudgeable is not new (O4).
    assert!(report.findings.iter().all(|f| f.check != "field-new"));
}

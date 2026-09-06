//! `doctor --for` judges a subject's declared `kind` (#422, RFC 08 §2
//! v1.32, RFC 13 §3) against a real bus: a counter that goes down is a
//! `kind-mismatch` finding naming its origin, a self-describing payload
//! whose tag disagrees is one too, a subject that declares no `kind` is
//! never judged, and a counter that resets across its producer's `alive`
//! cycle is not a finding — the restart is the one sanctioned reset, and
//! the listen phase watches the liveliness planes to see it.
//!
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::time::Duration;

use zenkey::qos::QosProfile;
use zenkey_fleet::report::CheckId;
use zenkey_fleet::{DoctorSpec, Fleet, declare_publication, run_doctor};

mod util;
use util::peer_pair;

fn spec(listen_s: u64) -> DoctorSpec {
    DoctorSpec {
        deep: false,
        sample: None,
        timeout: Duration::from_millis(500),
        listen: Some(Duration::from_secs(listen_s)),
    }
}

const KEY: &str = "v1/h-abababababab/telemetry/demo/oom_kills_total";

/// The subject under test declared as a counter…
const COUNTER_SLICE: &str = r#"
[registry]
version = "1.0"
app = "t"
convention = 1
[producer]
name = "demo"
[[subject]]
path = "oom_kills_total"
class = "telemetry"
type = "TelemetryPoint"
kind = "counter"
"#;

/// …and the same subject declaring nothing about its kind.
const UNKINDED_SLICE: &str = r#"
[registry]
version = "1.0"
app = "t"
convention = 1
[producer]
name = "demo"
[[subject]]
path = "oom_kills_total"
class = "telemetry"
type = "TelemetryPoint"
"#;

/// Publishes the bodies in a cycle, spaced, until dropped — so the listen
/// window sees the sequence whatever moment it opens at.
fn keep_cycling(
    publication: zenkey_fleet::Publication,
    bodies: &'static [&'static [u8]],
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        for body in bodies.iter().cycle().take(200) {
            if publication.send(body.to_vec(), None).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
}

fn kind_findings(
    report: &zenkey_fleet::report::DoctorReport,
) -> Vec<&zenkey_fleet::report::DoctorFinding> {
    report
        .findings
        .iter()
        .filter(|f| f.check == CheckId::KindMismatch)
        .collect()
}

/// A declared `counter` whose series decreases — 10, 20, 5 — with no
/// `alive` cycle of its origin in between: one finding, per origin, with
/// the window stated.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_decreasing_counter_is_a_kind_mismatch_finding() {
    let (a, b) = peer_pair().await;
    let local = zenkey::parse_slice(COUNTER_SLICE).expect("slice");

    let publication = declare_publication(&a, KEY, QosProfile::Sampled, None)
        .await
        .expect("declare");
    let t = keep_cycling(publication, &[b"10", b"20", b"5"]);

    let report = run_doctor(
        &Fleet::new(&b, ""),
        Some(&zenkey_fleet::SliceSet::from_slices(vec![local])),
        &spec(2),
    )
    .await
    .expect("run_doctor");
    t.abort();

    let obs = report.observation.as_ref().expect("observation ran");
    assert!(obs.samples > 0, "the window saw the fixture's traffic");

    let found = kind_findings(&report);
    assert_eq!(found.len(), 1, "{:?}", report.findings);
    let f = found[0];
    assert_eq!(f.severity, zenkey_fleet::report::DoctorSeverity::Error);
    assert_eq!(f.subject, KEY);
    assert!(
        f.evidence.contains("h-abababababab"),
        "per origin: {}",
        f.evidence
    );
    assert!(f.evidence.contains("decrease"), "{}", f.evidence);
    assert!(
        f.evidence.contains("2s"),
        "the window is stated: {}",
        f.evidence
    );
    assert_eq!(f.citation.as_deref(), Some("RFC 08 §2"));
}

/// A self-describing payload (RFC 11 §4's `TelemetryValue` shape) whose tag
/// says `gauge` on a subject declared `counter` is the disagreement RFC 08
/// §2 forbids — the value never decreases, and it is still a finding.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_gauge_tagged_payload_on_a_counter_subject_is_a_finding() {
    let (a, b) = peer_pair().await;
    let local = zenkey::parse_slice(COUNTER_SLICE).expect("slice");

    let publication = declare_publication(&a, KEY, QosProfile::Sampled, None)
        .await
        .expect("declare");
    let t = keep_cycling(publication, &[br#"{"type":"gauge","value":1}"#]);

    let report = run_doctor(
        &Fleet::new(&b, ""),
        Some(&zenkey_fleet::SliceSet::from_slices(vec![local])),
        &spec(2),
    )
    .await
    .expect("run_doctor");
    t.abort();

    let found = kind_findings(&report);
    assert_eq!(found.len(), 1, "{:?}", report.findings);
    assert_eq!(found[0].subject, KEY);
    assert!(
        found[0].evidence.contains("tags itself `gauge`"),
        "{}",
        found[0].evidence
    );
    assert!(
        found[0].evidence.contains("declared `counter`"),
        "{}",
        found[0].evidence
    );
}

/// The same decreasing traffic on a subject that declares no `kind` is not
/// asked — and not asked is never answered (RFC 13 §3, RFC 09 §5.1 O4).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_subject_without_kind_is_never_judged() {
    let (a, b) = peer_pair().await;
    let local = zenkey::parse_slice(UNKINDED_SLICE).expect("slice");

    let publication = declare_publication(&a, KEY, QosProfile::Sampled, None)
        .await
        .expect("declare");
    let t = keep_cycling(
        publication,
        &[b"10", b"20", b"5", br#"{"type":"gauge","value":1}"#],
    );

    let report = run_doctor(
        &Fleet::new(&b, ""),
        Some(&zenkey_fleet::SliceSet::from_slices(vec![local])),
        &spec(2),
    )
    .await
    .expect("run_doctor");
    t.abort();

    let obs = report.observation.as_ref().expect("observation ran");
    assert!(obs.samples > 0, "the window saw the fixture's traffic");
    assert!(
        kind_findings(&report).is_empty(),
        "an undeclared kind is not asked: {:?}",
        report.findings
    );
}

/// A counter that drops to zero right after its producer's `alive` token
/// cycled — down, then up — has restarted, which RFC 08 §2 sanctions; the
/// listen phase sees the cycle on the liveliness plane and resets the
/// baseline instead of reporting the drop.
///
/// The fixture repeats the whole cycle so the window sees at least one
/// wherever it opens: publish 10 a few times, undeclare the token, re-declare
/// it, publish 0 a few times, and again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_counter_reset_across_an_alive_cycle_is_not_a_finding() {
    let (a, b) = peer_pair().await;
    let local = zenkey::parse_slice(COUNTER_SLICE).expect("slice");

    const ALIVE: &str = "v1/h-abababababab/state/demo/alive";
    let publication = declare_publication(&a, KEY, QosProfile::Sampled, None)
        .await
        .expect("declare");
    let session = a.clone();
    let t = tokio::spawn(async move {
        let mut token = session
            .liveliness()
            .declare_token(ALIVE)
            .await
            .expect("token");
        for _ in 0..20 {
            for _ in 0..3 {
                if publication.send(b"10".to_vec(), None).await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            // The restart: the token cycles, and only then does the series
            // start over.
            token.undeclare().await.expect("undeclare token");
            tokio::time::sleep(Duration::from_millis(200)).await;
            token = session
                .liveliness()
                .declare_token(ALIVE)
                .await
                .expect("token again");
            tokio::time::sleep(Duration::from_millis(200)).await;
            for _ in 0..3 {
                if publication.send(b"0".to_vec(), None).await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    });

    let report = run_doctor(
        &Fleet::new(&b, ""),
        Some(&zenkey_fleet::SliceSet::from_slices(vec![local])),
        &spec(3),
    )
    .await
    .expect("run_doctor");
    t.abort();

    let obs = report.observation.as_ref().expect("observation ran");
    assert!(obs.samples > 0, "the window saw the fixture's traffic");
    assert!(
        kind_findings(&report).is_empty(),
        "a reset across an alive cycle is the sanctioned one: {:?}",
        report.findings
    );
}

//! `doctor --listen-for` (#161) against a real bus: the passive phase turns
//! observed traffic into typed findings, and the observation section states
//! window/scope/drops (O5/O6).
//!
//! The fixture publishers send continuously through the window, so no settle
//! is needed: the listen monitor subscribes, the next send arrives.
//! Ports 7535-7536 (disjoint from every other test binary).

use std::time::Duration;

use zenkey::qos::QosProfile;
use zenkey_fleet::{DoctorSpec, declare_publication, run_doctor};

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

fn spec(listen_s: u64) -> DoctorSpec {
    DoctorSpec {
        deep: false,
        sample: None,
        timeout: Duration::from_millis(500),
        listen: Some(Duration::from_secs(listen_s)),
    }
}

/// Keeps publishing until dropped — the fixture's way of being alive for
/// however long the listen window runs.
fn keep_publishing(
    publication: zenkey_fleet::Publication,
    body: &'static [u8],
    attachment: Option<Vec<u8>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        for _ in 0..100 {
            if publication
                .send(body.to_vec(), attachment.clone())
                .await
                .is_err()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
}

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
qos = "transition"
"#;

/// Wrong wire QoS on a registered subject and traffic on an undeclared one
/// each become their finding — aggregated per key, phrased per the RFC.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn observed_qos_and_unregistered_traffic_become_findings() {
    let (a, b) = peer_pair(7535).await;
    let local = zenkey::parse_slice(SLICE).expect("slice");

    // Declared transition, riding sampled: the mismatch under test.
    let wrong_qos = declare_publication(
        &a,
        "v1/h-abababababab/state/demo/health",
        QosProfile::Sampled,
        None,
    )
    .await
    .expect("declare");
    // A subject the slice does not declare.
    let unregistered = declare_publication(
        &a,
        "v1/h-abababababab/state/demo/undeclared",
        QosProfile::Transition,
        None,
    )
    .await
    .expect("declare");
    let t1 = keep_publishing(wrong_qos, b"{}", None);
    let t2 = keep_publishing(unregistered, b"{}", None);

    let report = run_doctor(&b, "", std::slice::from_ref(&local), &spec(2))
        .await
        .expect("run_doctor");
    t1.abort();
    t2.abort();

    let obs = report.observation.as_ref().expect("observation ran");
    assert!(obs.samples > 0, "the window saw the fixture's traffic");
    assert!(!obs.scopes.is_empty(), "coverage is a statement (O5)");

    let qos: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check == "qos-observed-mismatch")
        .collect();
    assert_eq!(qos.len(), 1, "{:?}", report.findings);
    assert_eq!(qos[0].subject, "v1/h-abababababab/state/demo/health");
    assert!(
        qos[0].evidence.contains("transition"),
        "{}",
        qos[0].evidence
    );
    assert_eq!(qos[0].citation.as_deref(), Some("RFC 04 §3"));

    let unreg: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check == "unregistered-traffic")
        .collect();
    assert_eq!(unreg.len(), 1, "{:?}", report.findings);
    assert_eq!(unreg[0].subject, "v1/h-abababababab/state/demo/undeclared");
}

const EVENTS_SLICE: &str = r#"
[registry]
version = "1.0"
app = "t"
convention = 1
[producer]
name = "demo"
[[subject]]
path = "boom/{id}"
class = "events"
type = "Boom"
rate = "rare"
"#;

/// Provable over-rate: more events inside the window than the declared class
/// allows per hour. And a synthetic-marked sample is counted apart — the
/// observation says how much of what it judged was generated (#162).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn over_rate_events_are_findings_and_synthetic_traffic_is_counted() {
    let (a, b) = peer_pair(7536).await;
    let local = zenkey::parse_slice(EVENTS_SLICE).expect("slice");

    let marker = br#"{"synthetic":true,"tool":"test"}"#.to_vec();
    let mut tasks = Vec::new();
    for (i, marked) in [(1, false), (2, false), (3, true)] {
        let publication = declare_publication(
            &a,
            // The ids only need to be distinct within the fixture.
            Box::leak(format!("v1/h-abababababab/events/demo/boom/01hq{i}").into_boxed_str()),
            QosProfile::Transition,
            None,
        )
        .await
        .expect("declare");
        tasks.push(keep_publishing(
            publication,
            b"{}",
            marked.then(|| marker.clone()),
        ));
    }

    let report = run_doctor(&b, "", std::slice::from_ref(&local), &spec(2))
        .await
        .expect("run_doctor");
    for t in tasks {
        t.abort();
    }

    let obs = report.observation.as_ref().expect("observation ran");
    assert!(
        obs.synthetic_marked > 0,
        "the marked stream is counted apart: {obs:?}"
    );

    let over: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check == "rate-over-declared")
        .collect();
    assert_eq!(over.len(), 1, "{:?}", report.findings);
    assert_eq!(over[0].subject, "demo/boom/{id}");
    assert!(over[0].evidence.contains("rare"), "{}", over[0].evidence);
    assert_eq!(over[0].citation.as_deref(), Some("RFC 04 §1.3"));
}

//! `run_expect` (#160): the three-state verdict, live on a bus.
//!
//! Event-driven settles: the publisher's matching badge proves the expect
//! window's subscriber is routable before anything is sent — a fixture that
//! publishes into the void tests the void.
//! Ports 7532-7533 and 7535-7536 (disjoint from every other test binary).

use std::time::Duration;

use zenkey::qos::QosProfile;
use zenkey_fleet::report::ExpectVerdict;
use zenkey_fleet::{ExpectSpec, QosCheck, declare_publication, run_expect};

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

const KEY: &str = "v1/h-cccccccccccc/state/demo/health";

fn spec(selector: &str, within_s: f64) -> ExpectSpec {
    ExpectSpec {
        selector: selector.to_string(),
        within: Duration::from_secs_f64(within_s),
        ..ExpectSpec::default()
    }
}

/// A sample that arrives meets the default existence expectation early; a
/// count the window never fills is NOT MET on a clean observation — the two
/// exits CI most needs to trust.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn presence_meets_early_and_a_clean_shortfall_is_not_met() {
    let (a, b) = peer_pair(7532).await;
    let slices = zenkey_fleet::SliceSet::default();

    let publication = declare_publication(&a, KEY, QosProfile::Transition, None)
        .await
        .expect("declare");
    let matching = publication.matching_events().await.expect("events");

    let expect = tokio::spawn({
        let b = b.clone();
        async move {
            let slices = zenkey_fleet::SliceSet::default();
            run_expect(&b, "", &slices, &store_of(), &spec(KEY, 10.0)).await
        }
    });

    // The expect window's subscriber raises the badge; then publish into it.
    assert!(
        tokio::time::timeout(Duration::from_secs(5), matching.recv())
            .await
            .expect("matching within 5s")
            .expect("listener alive")
    );
    publication.send(b"{}".to_vec(), None).await.expect("send");

    let report = expect.await.expect("join").expect("run");
    assert_eq!(report.verdict, ExpectVerdict::Met);
    assert!(report.ended_early, "one sample satisfies the default count");
    assert_eq!(report.samples, 1);
    assert_eq!(report.dropped, 0);

    // Now demand more than the window will carry: clean shortfall → NotMet.
    let shortfall = ExpectSpec {
        count: Some(3),
        ..spec(KEY, 1.0)
    };
    let report = run_expect(&b, "", &slices, &store_of(), &shortfall)
        .await
        .expect("run");
    assert_eq!(report.verdict, ExpectVerdict::NotMet);
    assert!(
        report.unmet.iter().any(|u| u.contains("3 required")),
        "{:?}",
        report.unmet
    );
}

/// `absent` holds on a quiet selector — and one sample where none may be is
/// conclusive NOT MET, not a shrug.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn absence_is_scoped_clean_and_conclusively_breakable() {
    let (a, b) = peer_pair(7533).await;
    let slices = zenkey_fleet::SliceSet::default();

    let quiet = ExpectSpec {
        absent: true,
        ..spec("v1/*/state/demo/retired", 1.0)
    };
    let report = run_expect(&b, "", &slices, &store_of(), &quiet)
        .await
        .expect("run");
    assert_eq!(report.verdict, ExpectVerdict::Met);
    assert_eq!(report.samples, 0);
    assert_eq!(report.dropped, 0, "the claim states its observer was clean");

    // Break it: a publisher on the supposedly-retired key.
    let publication = declare_publication(&a, KEY, QosProfile::Transition, None)
        .await
        .expect("declare");
    let matching = publication.matching_events().await.expect("events");
    let broken = ExpectSpec {
        absent: true,
        ..spec(KEY, 5.0)
    };
    let expect = tokio::spawn({
        let b = b.clone();
        async move {
            let slices = zenkey_fleet::SliceSet::default();
            run_expect(&b, "", &slices, &store_of(), &broken).await
        }
    });
    assert!(
        tokio::time::timeout(Duration::from_secs(5), matching.recv())
            .await
            .expect("matching within 5s")
            .expect("listener alive")
    );
    publication.send(b"{}".to_vec(), None).await.expect("send");

    let report = expect.await.expect("join").expect("run");
    assert_eq!(report.verdict, ExpectVerdict::NotMet);
    assert_eq!(report.violations_total, 1);
    assert!(
        report.violations[0].contains("none may be"),
        "{:?}",
        report.violations
    );
}

/// `--qos declared`: a sample riding its declared profile passes; one riding
/// anything else is a named violation (#159's read-side sibling).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declared_qos_is_checked_per_sample() {
    let slice = zenkey::parse_slice(
        r#"
[registry]
version = "1.0"
app = "demo"
convention = 1
[producer]
name = "demo"
[[subject]]
path = "health"
class = "state"
type = "Health"
qos = "transition"
"#,
    )
    .expect("fixture slice parses");
    let slices = zenkey_fleet::SliceSet::from_slices(vec![slice]);

    // A pair per run, not one pair for both (#231). The settle below waits for
    // the publisher's matching listener, which proves *a* subscriber is
    // routable — it cannot prove it is *this* run's. Sharing the sessions let
    // the second run match the first run's subscriber, whose undeclare had not
    // yet propagated, and send into the gap before its own window subscribed.
    // Under load that lost the sample ~22% of the time, and the test then
    // failed on an empty violation list rather than on the QoS it checks.
    let run_with = |port: u16, qos_profile: QosProfile, slices: zenkey_fleet::SliceSet| async move {
        let (a, b) = peer_pair(port).await;
        let publication = declare_publication(&a, KEY, qos_profile, None)
            .await
            .expect("declare");
        let matching = publication.matching_events().await.expect("events");
        let spec = ExpectSpec {
            qos: Some(QosCheck::Declared),
            ..spec(KEY, 5.0)
        };
        let expect = tokio::spawn({
            let b = b.clone();
            async move { run_expect(&b, "", &slices, &store_of(), &spec).await }
        });
        assert!(
            tokio::time::timeout(Duration::from_secs(5), matching.recv())
                .await
                .expect("matching within 5s")
                .expect("listener alive")
        );
        publication.send(b"{}".to_vec(), None).await.expect("send");
        expect.await.expect("join").expect("run")
    };

    let report = run_with(7535, QosProfile::Transition, slices.clone()).await;
    assert_eq!(report.verdict, ExpectVerdict::Met, "{:?}", report.unmet);

    let report = run_with(7536, QosProfile::Sampled, slices).await;
    assert_eq!(report.verdict, ExpectVerdict::NotMet);
    // Named, not indexed: an empty list here means the window saw no samples
    // at all — a presence failure, not the QoS failure under test — and
    // `violations[0]` threw that diagnosis away behind an index panic (#231).
    assert_eq!(
        report.violations.len(),
        1,
        "expected one QoS violation; an empty list means the window observed \
         no samples, which is a different failure: unmet={:?}",
        report.unmet
    );
    assert!(
        report.violations[0].contains("did not ride transition"),
        "{:?}",
        report.violations
    );
}

fn store_of() -> zenkey_fleet::decode::SchemaStore {
    zenkey_fleet::decode::SchemaStore::new("", Duration::from_millis(300))
}

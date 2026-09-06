//! The RPC trace window against a real bus (#215): a producer that follows
//! RFC 05 §3's long-running idiom — reply `{id}` on its own key, then a
//! status state, then an audit event — traced end to end, with a noisy
//! subject on the same origin and a second origin publishing alongside.
//!
//! What this pins: the reply precedes both effects; the attributed lane is
//! the state then the event, in arrival order, with both clocks present
//! (the producer's session stamps its publications, and its driver stamps
//! the reply — zenoh's timestamping does not — so the Δ is computable); the noise never enters the attributed
//! lane; the other origin is only ever a count. Registry attribution comes
//! from a hand-built slice — `SliceSet::from_slices` — so the test depends
//! on no registry directory.
//!
//! **A zenctl-level smoke cannot show this.** `gen --serve-describe`
//! answers queries and never publishes *after* a reply, so a trace against
//! the generator shows naming attribution over its steady traffic only;
//! the ordered chain needs a producer whose driver publishes in response
//! to the query, which is what the task below is.

use std::time::Duration;

use zenkey::slice::{ProcedureDecl, RegistrySlice, SubjectDecl};
use zenkey_fleet::report::{HlcReference, TraceRelation};
use zenkey_fleet::{BringUp, CallSpec, CallTarget, Fleet, SliceSet, TraceSpec, call_traced};

mod util;
use util::timestamping_pair;

const ORIGIN: &str = "h-aaaaaaaaaaaa";
const OTHER: &str = "h-bbbbbbbbbbbb";

fn slices() -> SliceSet {
    let mut slice = RegistrySlice::new("1.0", "t", "demo");
    let mut request = ProcedureDecl::new("artifact/request");
    request.kind = Some(zenkey::Declared::Other("long-running".into()));
    slice.procedures = vec![request];
    slice.subjects = vec![
        SubjectDecl::new("artifact/{kind}", zenkey::Class::State),
        SubjectDecl::new("artifact/{ulid}", zenkey::Class::Events),
    ];
    SliceSet::from_slices(vec![slice])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_traced_long_running_call_lists_request_then_state_then_event_on_both_clocks() {
    let (producer, explorer) = timestamping_pair().await;
    let rpc_key = format!("v1/{ORIGIN}/@rpc/demo/artifact/request");
    let state_key = format!("v1/{ORIGIN}/state/demo/artifact/pcap");
    let event_key = format!("v1/{ORIGIN}/events/demo/artifact/01HZY");
    let noise_key = format!("v1/{ORIGIN}/telemetry/other/noise");
    let other_key = format!("v1/{OTHER}/telemetry/sysinfo/cpu");

    // The producer's publications are declared **before** any query
    // arrives — the effects ride publishers that already exist, so the
    // first put after the reply is routable.
    let state_pub = producer
        .declare_publisher(state_key.clone())
        .await
        .expect("state publisher");
    let event_pub = producer
        .declare_publisher(event_key.clone())
        .await
        .expect("event publisher");
    let noise_pub = producer
        .declare_publisher(noise_key.clone())
        .await
        .expect("noise publisher");
    let other_pub = producer
        .declare_publisher(other_key.clone())
        .await
        .expect("other-origin publisher");

    // Bring the producer up in RFC 04 §5 order and drive it: per query,
    // reply `{id}` on its own key, then publish the status state and the
    // audit event — the idiom, in the idiom's order.
    let mut up = BringUp::new(&producer);
    up.serve(&rpc_key).await.expect("declare the procedure");
    let responders = up.without_alive();
    let stamper = producer.clone();
    let driver = tokio::spawn(async move {
        let responder = &responders[0];
        while let Some(query) = responder.next().await {
            // The reply is stamped by the producer, deliberately: a
            // deployment's timestamping stamps publications, not replies,
            // and the reply's HLC is what the Δ column measures from.
            let ts = stamper.new_timestamp();
            responder
                .reply_stamped(
                    &query,
                    br#"{"id":"01HZY"}"#.to_vec(),
                    Some("application/json"),
                    Some(ts),
                )
                .await
                .expect("reply");
            // A beat between reply and effect, so the arrival order is
            // not a race between two puts on one thread.
            tokio::time::sleep(Duration::from_millis(30)).await;
            state_pub
                .put(br#"{"status":"generating"}"#.to_vec())
                .await
                .expect("state put");
            tokio::time::sleep(Duration::from_millis(30)).await;
            event_pub
                .put(br#"{"id":"01HZY","status":"ready"}"#.to_vec())
                .await
                .expect("event put");
        }
    });
    // Same-origin noise at 50 Hz, and one other origin, throughout.
    let noise = tokio::spawn(async move {
        loop {
            noise_pub.put(vec![1]).await.expect("noise put");
            other_pub.put(vec![2]).await.expect("other put");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });

    // Prove routability first: a GET before the peers connect is silence,
    // not evidence.
    let fleet = Fleet::new(&explorer, "");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let answers = zenkey_fleet::fleet_get(
            &fleet,
            &rpc_key,
            &zenkey_fleet::GetOpts::new(Duration::from_millis(500)),
        )
        .await
        .expect("probe");
        if !answers.is_empty() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the procedure never became routable"
        );
    }

    let target = CallTarget::parse(ORIGIN).expect("target");
    let slices = slices();
    let report = call_traced(
        &fleet,
        CallSpec {
            target: &target,
            producer: "demo",
            procedure: "artifact/request",
            params: &[],
            body: Some(br#"{"kind":"pcap"}"#.to_vec()),
            attachment: None,
            timeout: Duration::from_secs(5),
            slices: Some(&slices),
        },
        TraceSpec {
            window: Duration::from_secs(2),
        },
    )
    .await
    .expect("traced call");
    noise.abort();
    driver.abort();

    // The reply, first and alone: one origin answered, with the id.
    assert_eq!(report.call.exit_code(), 0);
    assert_eq!(report.call.answers.len(), 1);
    assert_eq!(report.call.answers[0].origin, ORIGIN);
    assert_eq!(report.idiom, "long-running");
    assert!(report.subscribed_before_call);
    assert!(report.registry_loaded);
    assert_eq!(
        report.scopes,
        [format!("v1/{ORIGIN}/**"), "v1/*/**".to_string()]
    );

    // The producer's session stamps, so the reply carried an HLC and the
    // Δ column exists.
    assert_eq!(report.hlc_reference, HlcReference::Reply);
    assert!(report.reply_hlc.is_some());

    // The attributed lane: state, then event, in arrival order, both in
    // the declared chain, both stamped, both after the call returned.
    let keys: Vec<&str> = report.attributed.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(
        keys,
        [state_key.as_str(), event_key.as_str()],
        "{report:#?}"
    );
    for r in &report.attributed {
        assert_eq!(r.relation, TraceRelation::DeclaredChain);
        assert!(r.hlc.is_some(), "the producer stamps: {r:?}");
        assert!(r.hlc_delta_ms.is_some(), "both clocks present: {r:?}");
        assert!(r.stamped_by.is_some());
        // "The reply precedes the effect" is read on the stamper's clock,
        // not against `call_returned_ms`: the fan-in returns when the query
        // *finalizes*, which is after the reply and — as this test shows —
        // often after the producer's first effect has already arrived. The
        // report documents that instant as an upper bound for exactly this
        // reason.
        assert!(
            r.hlc_delta_ms.unwrap() > 0,
            "an effect is stamped after the reply on the producer's clock: {r:?}"
        );
    }
    assert!(
        report.attributed[0].arrival_delta_ms < report.attributed[1].arrival_delta_ms,
        "state before event on the arrival clock"
    );
    assert!(
        report.attributed[0].hlc_delta_ms.unwrap() <= report.attributed[1].hlc_delta_ms.unwrap(),
        "state before event on the stamper's clock"
    );

    // The noise: same origin, never in the chain — and present, because
    // it ran the whole window.
    assert!(
        report
            .same_origin
            .iter()
            .all(|r| r.key == noise_key && r.relation == TraceRelation::SameOriginUndeclared),
        "{:?}",
        report.same_origin
    );
    assert!(
        !report.same_origin.is_empty(),
        "50 Hz for 2 s is not silence"
    );

    // The other origin: a count and an example, never a row anywhere.
    assert!(report.concurrent.samples > 0);
    assert_eq!(report.concurrent.keys, 1);
    assert_eq!(report.concurrent.examples, std::slice::from_ref(&other_key));
    assert!(
        report
            .attributed
            .iter()
            .chain(&report.same_origin)
            .all(|r| !r.key.starts_with(&format!("v1/{OTHER}/"))),
        "a busy unrelated origin never appears in an origin lane"
    );
}

/// A fleet target has nothing to attribute to, and the engine says so
/// before any watch is declared.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fleet_target_cannot_be_traced() {
    let (_producer, explorer) = timestamping_pair().await;
    let fleet = Fleet::new(&explorer, "");
    let err = call_traced(
        &fleet,
        CallSpec {
            target: &CallTarget::Fleet,
            producer: "demo",
            procedure: "artifact/request",
            params: &[],
            body: None,
            attachment: None,
            timeout: Duration::from_secs(1),
            slices: None,
        },
        TraceSpec {
            window: Duration::from_millis(100),
        },
    )
    .await
    .unwrap_err();
    assert!(err.is_unaskable(), "{err}");
    assert!(err.to_string().contains("name one origin"), "{err}");
}

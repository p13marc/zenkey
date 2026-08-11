//! The mock responder (#121): a declared queryable that answers with static
//! bytes and surfaces every ask. Ports 7520-7521 (disjoint from every other
//! test binary).

use std::time::Duration;

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

const KEY: &str = "v1/h-ffffffffffff/@rpc/mock/answer";

/// A GET through the fleet chokepoint reaches the responder, gets the static
/// body back on the query's own key, and the ask surfaces in the log view —
/// parameters and query body included.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_responder_answers_and_logs_the_ask() {
    let (a, b) = peer_pair(7520).await;

    let responder = zenkey_fleet::declare_responder(
        &a,
        KEY,
        br#"{"mock":true}"#.to_vec(),
        Some("application/json"),
        false,
    )
    .await
    .expect("declare responder");

    let log = tokio::spawn(async move {
        let view = responder.next().await.expect("one ask");
        responder.undeclare().await.expect("undeclare");
        view
    });

    // Settle: loop the GET until the queryable answers (wait-routable).
    let answers = loop {
        let answers = zenkey_fleet::fleet_get(
            &b,
            "",
            &format!("{KEY}?who=test"),
            Some(b"ping".to_vec()),
            Duration::from_millis(500),
        )
        .await
        .expect("get");
        if !answers.is_empty() {
            break answers;
        }
    };
    let zenkey_fleet::Answer::Value(v) = &answers[0].answer else {
        panic!("expected a value reply");
    };
    assert_eq!(v.to_bytes().as_ref(), br#"{"mock":true}"#);
    assert_eq!(answers[0].key, KEY, "replied on the query's own key");
    assert_eq!(answers[0].encoding.as_deref(), Some("application/json"));

    let view = log.await.expect("join");
    assert!(view.parameters.contains("who=test"), "{}", view.parameters);
    assert_eq!(
        view.payload
            .expect("query body surfaced")
            .to_bytes()
            .as_ref(),
        b"ping"
    );
}

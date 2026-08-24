//! The mock responder (#121): a declared queryable that answers with static
//! bytes and surfaces every ask.
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::time::Duration;

mod util;
use util::peer_pair;

const KEY: &str = "v1/h-ffffffffffff/@rpc/mock/answer";

/// A GET through the fleet chokepoint reaches the responder, gets the static
/// body back on the query's own key, and the ask surfaces in the log view —
/// parameters and query body included.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_responder_answers_and_logs_the_ask() {
    let (a, b) = peer_pair().await;

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
            &zenkey_fleet::Fleet::new(&b, ""),
            &format!("{KEY}?who=test"),
            &zenkey_fleet::GetOpts::new(Duration::from_millis(500)).payload(Some(b"ping".to_vec())),
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
    assert_eq!(
        answers[0].key, KEY,
        "replied on the responder's own concrete key (RFC 05 §2.1)"
    );
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
    assert_eq!(
        view.reply_error, None,
        "a sent reply carries no error (deep-review D7: the error path \
         rides the view, not the void)"
    );
}

/// G-05b, the fix pinned from the wire: a **wildcard** GET at a concrete
/// responder gets its reply on the responder's own concrete key — before
/// the fix, the mock echoed `query.key_expr()` and the reply key was the
/// caller's wildcard selector, which default consolidation would collapse
/// across a mocked fleet (RFC 05 §2.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wildcard_ask_is_answered_on_the_responders_concrete_key() {
    let (a, b) = peer_pair().await;

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

    let answers = loop {
        let answers = zenkey_fleet::fleet_get(
            &zenkey_fleet::Fleet::new(&b, ""),
            "v1/*/@rpc/mock/answer",
            &zenkey_fleet::GetOpts::new(Duration::from_millis(500)),
        )
        .await
        .expect("get");
        if !answers.is_empty() {
            break answers;
        }
    };
    assert_eq!(
        answers[0].key, KEY,
        "the reply rides the responder's concrete key, not the echoed \
         wildcard selector (RFC 05 §2.1, G-05b)"
    );
    let view = log.await.expect("join");
    assert!(view.selector.contains("v1/*"), "{}", view.selector);
}

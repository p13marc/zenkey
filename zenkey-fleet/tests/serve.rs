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
        let query = responder.next().await.expect("one ask");
        let view = responder.answer(query).await;
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
        let query = responder.next().await.expect("one ask");
        let view = responder.answer(query).await;
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

/// #333: receiving and answering are separate awaits, so a window that closes
/// between them loses neither the reply nor the log line.
///
/// The old `next()` had an await at each end — `recv_async` took the query,
/// `reply` sent it — and a caller dropped in between consumed a query that was
/// then never answered and never logged: silence the asker cannot attribute
/// (RFC 05 §3.1) and an ask the responder's own log never records
/// (RFC 13 §3 O6). Today `ctrl_c` is the only thing racing that loop, which is
/// benign; the moment a `--for` deadline joins it, this is the shape that
/// stays honest.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_query_in_hand_outlives_the_window_that_took_it() {
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
        let query = responder.next().await.expect("one ask");
        // The window expires *here*, with the query already off the channel —
        // precisely where the combined call used to lose it. Answering is a
        // second await the caller owns, so the deadline cannot cut it short.
        let expired = tokio::select! {
            _ = tokio::time::sleep(Duration::ZERO) => true,
            _ = std::future::pending::<()>() => false,
        };
        assert!(expired, "the window closed while the query was in hand");
        let view = responder.answer(query).await;
        responder.undeclare().await.expect("undeclare");
        view
    });

    let answers = loop {
        let answers = zenkey_fleet::fleet_get(
            &zenkey_fleet::Fleet::new(&b, ""),
            KEY,
            &zenkey_fleet::GetOpts::new(Duration::from_millis(500)),
        )
        .await
        .expect("get");
        if !answers.is_empty() {
            break answers;
        }
    };
    let zenkey_fleet::Answer::Value(v) = &answers[0].answer else {
        panic!("the reply still went out");
    };
    assert_eq!(v.to_bytes().as_ref(), br#"{"mock":true}"#);

    let view = log.await.expect("join");
    assert_eq!(view.reply_error, None, "the reply left");
    assert!(view.selector.contains(KEY), "the ask reached the log");
}

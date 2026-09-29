//! Producer bring-up (RFC 04 §5) over a real pair of sessions: queryables
//! before `alive`, replies on the concrete key, reserved-error refusals on
//! `reply_err`.
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::time::Duration;

use zenkey_fleet::bus::producer::{BringUp, ReservedError};

mod util;
use util::peer_pair;

const INTROSPECT: &str = "v1/h-abcdefabcdef/@rpc/mockp/introspect";
const ALIVE: &str = "v1/h-abcdefabcdef/state/mockp/alive";

/// The ordered bring-up observed from the consumer side: once the `alive`
/// token is visible on the roster, the queryable is already callable —
/// "alive ⇒ callable" (RFC 04 §5) — and the value reply rides the
/// producer's own concrete key. A gated refusal rides `reply_err` with the
/// reserved name (RFC 05 §3, RFC 08 §6.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn alive_implies_callable_and_replies_ride_the_concrete_key() {
    let (a, b) = peer_pair().await;

    // A wildcard queryable is not a producer posture: refused up front.
    let mut up = BringUp::new(&a);
    let err = up
        .serve("v1/h-abcdefabcdef/@rpc/mockp/*")
        .await
        .expect_err("wildcards refused")
        .to_string();
    assert!(err.contains("concrete"), "{err}");

    up.serve(INTROSPECT).await.expect("declare queryable");
    let live = up.alive(ALIVE).await.expect("declare alive last");

    // Drive the responder: answer one query with a value on the concrete
    // key, the next with a reserved-name refusal.
    let serving = tokio::spawn(async move {
        let responder = &live.responders[0];
        let q = responder.next().await.expect("first ask");
        responder
            .reply(&q, b"[]".to_vec(), Some("application/json"))
            .await
            .expect("value reply");
        let q = responder.next().await.expect("second ask");
        responder
            .reply_err(&q, ReservedError::Gated, "disabled by operator policy")
            .await
            .expect("error reply");
        live.retire().await.expect("retire");
    });

    // Wait for the token: the roster half of "alive ⇒ callable".
    loop {
        let Ok(replies) = b
            .liveliness()
            .get("v1/*/state/*/alive")
            .timeout(Duration::from_millis(500))
            .await
        else {
            continue;
        };
        let mut seen = false;
        while let Ok(reply) = replies.recv_async().await {
            if let Ok(sample) = reply.into_result() {
                seen |= sample.key_expr().as_str() == ALIVE;
            }
        }
        if seen {
            break;
        }
    }

    // Alive was visible, so the call must be answerable *now* — one GET,
    // no settle loop: that is the invariant the ordering exists for.
    let answers = zenkey_fleet::fleet_get(
        &zenkey_fleet::Fleet::new(&b, ""),
        INTROSPECT,
        &zenkey_fleet::GetOpts::new(Duration::from_secs(5)),
    )
    .await
    .expect("get");
    // (The reply set may also carry the transport's own timeout marker at
    // its close — the value answer is the one attributed to the producer.)
    let value: Vec<_> = answers
        .iter()
        .filter(|a| matches!(a.answer, zenkey_fleet::Answer::Value(_)))
        .collect();
    assert_eq!(value.len(), 1, "alive ⇒ callable (RFC 04 §5): {answers:?}");
    assert_eq!(
        value[0].key, INTROSPECT,
        "the reply rides the producer's own concrete key (RFC 05 §2.1)"
    );
    let zenkey_fleet::Answer::Value(v) = &value[0].answer else {
        unreachable!()
    };
    assert_eq!(v.to_bytes().as_ref(), b"[]");

    // The refusal: an error reply carrying the reserved name, never a
    // success payload with `ok: false` (RFC 05 §3) — and the caller's
    // chokepoint parses the envelope back out.
    let answers = zenkey_fleet::fleet_get(
        &zenkey_fleet::Fleet::new(&b, ""),
        INTROSPECT,
        &zenkey_fleet::GetOpts::new(Duration::from_secs(5)),
    )
    .await
    .expect("get");
    let gated: Vec<_> = answers
        .iter()
        .filter_map(|a| match &a.answer {
            zenkey_fleet::Answer::Error { name, message } => Some((name, message)),
            zenkey_fleet::Answer::Value(_) => None,
        })
        .filter(|(name, _)| *name == ReservedError::Gated.name())
        .collect();
    assert_eq!(gated.len(), 1, "one gated refusal: {answers:?}");
    assert_eq!(gated[0].1, "disabled by operator policy");

    serving.await.expect("join");
}

/// The reserved vocabulary is closed and spelled by the enum — seven names,
/// each namespaced like a key (RFC 05 §3; the seventh since v1.38, §2.1).
#[test]
fn reserved_names_are_the_rfcs() {
    let names: Vec<&str> = ReservedError::ALL.iter().map(|e| e.name()).collect();
    assert_eq!(
        names,
        [
            "error/invalid-args",
            "error/unauthorized",
            "error/not-found",
            "error/unsupported",
            "error/busy",
            "error/gated",
            "error/fanout-forbidden",
        ]
    );
    // And read back by the same enum (#222): every name round-trips, and a
    // producer's own registered name is not one of the seven.
    for e in ReservedError::ALL {
        assert_eq!(ReservedError::parse(e.name()), Some(e));
    }
    assert_eq!(ReservedError::parse("error/modem/restart-required"), None);
    assert_eq!(ReservedError::parse("gated"), None);
}

const SET: &str = "v1/h-abcdefabcdef/@rpc/mockp/knob/set";
const SET_ALIVE: &str = "v1/h-abcdefabcdef/state/mockp2/alive";

/// Wait until the roster shows `alive`: the "alive ⇒ callable" half of
/// RFC 04 §5, so the calls below need no settle loop of their own.
async fn wait_alive(consumer: &zenoh::Session, alive: &str) {
    loop {
        let Ok(replies) = consumer
            .liveliness()
            .get("v1/*/state/*/alive")
            .timeout(Duration::from_millis(500))
            .await
        else {
            continue;
        };
        let mut seen = false;
        while let Ok(reply) = replies.recv_async().await {
            if let Ok(sample) = reply.into_result() {
                seen |= sample.key_expr().as_str() == alive;
            }
        }
        if seen {
            break;
        }
    }
}

/// RFC 05 §2.1 (v1.38): a write declared through `serve_write` answers only
/// its own concrete key. A broadcast sent the careless way — a raw
/// `session.get` with a wildcard, no builder and no fleet chokepoint in the
/// way — is refused `error/fanout-forbidden` **by the server**, and the
/// handler never sees it; the exact call is served as before.
///
/// The ACL could not have done this: a deny rule fires only when it
/// *includes* the query's key expression (RFC 09 §3 fact 6), and
/// `v1/*/@rpc/mockp/**` is broader than any rule a producer would write.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_broadcast_write_is_refused_at_the_server_and_never_reaches_the_handler() {
    let (a, b) = peer_pair().await;

    let mut up = BringUp::new(&a);
    up.serve_write(SET).await.expect("declare the write");
    let live = up.alive(SET_ALIVE).await.expect("declare alive last");

    let serving = tokio::spawn(async move {
        let responder = &live.responders[0];
        // The first query the handler sees is the exact one: the broadcast
        // before it was answered and dropped inside `next`.
        let q = responder.next().await.expect("the exact ask");
        assert_eq!(
            q.key_expr().as_str(),
            SET,
            "a write handler sees only its own concrete key (RFC 05 §2.1)"
        );
        responder
            .reply(&q, b"\"applied\"".to_vec(), Some("application/json"))
            .await
            .expect("value reply");
        live.retire().await.expect("retire");
    });

    wait_alive(&b, SET_ALIVE).await;

    // The broadcast: one mistargeted `*` that would actuate every host at
    // once, if anything let it through.
    let replies = b
        .get("v1/*/@rpc/mockp/knob/set")
        .target(zenoh::query::QueryTarget::All)
        .timeout(Duration::from_secs(5))
        .await
        .expect("get");
    let mut refusals = Vec::new();
    let mut values = 0;
    while let Ok(reply) = replies.recv_async().await {
        match reply.into_result() {
            Ok(_) => values += 1,
            Err(e) => {
                let envelope: serde_json::Value =
                    serde_json::from_slice(&e.payload().to_bytes()).expect("an RFC 05 §3 envelope");
                refusals.push(envelope);
            }
        }
    }
    assert_eq!(values, 0, "a broadcast write must never be served");
    assert_eq!(
        refusals.len(),
        1,
        "one refusal, from the one producer: {refusals:?}"
    );
    assert_eq!(refusals[0]["error"], ReservedError::FanoutForbidden.name());
    let message = refusals[0]["message"].as_str().expect("a message");
    assert!(
        message.contains("v1/*/@rpc/mockp/knob/set") && message.contains(SET),
        "the message names both keys: {message}"
    );

    // The exact call, through the chokepoint, is served.
    let answers = zenkey_fleet::fleet_get(
        &zenkey_fleet::Fleet::new(&b, ""),
        SET,
        &zenkey_fleet::GetOpts::new(Duration::from_secs(5)),
    )
    .await
    .expect("get");
    let value: Vec<_> = answers
        .iter()
        .filter(|a| matches!(a.answer, zenkey_fleet::Answer::Value(_)))
        .collect();
    assert_eq!(value.len(), 1, "the exact call is answered: {answers:?}");

    serving.await.expect("join");
}

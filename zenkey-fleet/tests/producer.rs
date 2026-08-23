//! Producer bring-up (RFC 04 §5) over a real pair of sessions: queryables
//! before `alive`, replies on the concrete key, reserved-error refusals on
//! `reply_err`. Ports 7550-7551 (disjoint from every other test binary).

use std::time::Duration;

use zenkey_fleet::producer::{BringUp, ReservedError};

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

const INTROSPECT: &str = "v1/h-abcdefabcdef/@rpc/mockp/introspect";
const ALIVE: &str = "v1/h-abcdefabcdef/state/mockp/alive";

/// The ordered bring-up observed from the consumer side: once the `alive`
/// token is visible on the roster, the queryable is already callable —
/// "alive ⇒ callable" (RFC 04 §5) — and the value reply rides the
/// producer's own concrete key. A gated refusal rides `reply_err` with the
/// reserved name (RFC 05 §3, RFC 08 §6.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn alive_implies_callable_and_replies_ride_the_concrete_key() {
    let (a, b) = peer_pair(7550).await;

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
    let answers = zenkey_fleet::fleet_get(&b, "", INTROSPECT, None, Duration::from_secs(5))
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
    let answers = zenkey_fleet::fleet_get(&b, "", INTROSPECT, None, Duration::from_secs(5))
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

/// The reserved vocabulary is closed and spelled by the enum — six names,
/// each namespaced like a key (RFC 05 §3).
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
        ]
    );
}

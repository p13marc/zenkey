//! Call-side attachments (#126): the query carries one out, the reply
//! carries one back, and the projection into `CallAnswer` keeps it —
//! present only when the wire carried one, absent otherwise (O4).
//!
//! Port 7526 (disjoint from every other test binary).

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

const RPC_KEY: &str = "v1/h-aaaaaaaaaaaa/@rpc/demo/echo";

/// An echoing responder: the reply's attachment is the query's, so one
/// round trip proves both halves — the send path put it on the wire, and
/// the reply path carried it back into the report.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_carries_attachments_both_ways() {
    let (server, client) = peer_pair(7526).await;

    let queryable = server
        .declare_queryable(RPC_KEY)
        .await
        .expect("declare queryable");
    tokio::spawn(async move {
        while let Ok(query) = queryable.recv_async().await {
            let mut reply = query.reply(RPC_KEY, br#"{"ok":true}"#.to_vec());
            if let Some(att) = query.attachment() {
                reply = reply.attachment(att.to_bytes().to_vec());
            }
            let _ = reply.await;
        }
    });

    // Prove routability before asserting on attachment semantics: a query
    // issued before the peers connect answers nothing, which is silence,
    // not evidence.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let answers =
            zenkey_fleet::fleet_get(&client, "", RPC_KEY, None, Duration::from_millis(500))
                .await
                .expect("probe");
        if !answers.is_empty() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "responder never became routable"
        );
    }

    let target = zenkey_fleet::CallTarget::parse("h-aaaaaaaaaaaa").expect("target");
    let report = zenkey_fleet::call(
        &client,
        "",
        &target,
        "demo",
        "echo",
        &[],
        None,
        Some(b"who=me".to_vec()),
        Duration::from_secs(5),
        None,
    )
    .await
    .expect("call");
    assert_eq!(report.answers.len(), 1);
    let a = &report.answers[0];
    assert!(a.ok);
    assert_eq!(
        a.attachment,
        Some(serde_json::Value::String("who=me".into())),
        "the reply attachment reaches the report, projected verbatim"
    );
    assert_eq!(a.attachment_bytes, Some(6));

    // And a call sending none gets none back: absent, never defaulted.
    let report = zenkey_fleet::call(
        &client,
        "",
        &target,
        "demo",
        "echo",
        &[],
        None,
        None,
        Duration::from_secs(5),
        None,
    )
    .await
    .expect("call without attachment");
    assert_eq!(report.answers.len(), 1);
    assert_eq!(report.answers[0].attachment, None);
    assert_eq!(report.answers[0].attachment_bytes, None);
}
